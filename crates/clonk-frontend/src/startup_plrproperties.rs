//! Controller and classic renderer for the startup player-properties dialog.
//!
//! Persistence deliberately stays outside this module.  The controller owns
//! the editable `PlayerFile`, previews and the three-way image update intent
//! needed by the application to distinguish an unchanged image from a cleared
//! one.

use crate::classic_gui::{ClassicButtonState, ClassicGuiSkin, IntRect};
use crate::startup_main_menu::StartupTooltip;
use crate::startup_portraitsel::{
    PortraitFileEntry, PortraitLocation, PortraitSelAction, PortraitSelCommit,
    PortraitSelController, PortraitSelResources, PortraitThumbnailRequest,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};
use clonk_engine::player_file::PlayerFile;
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{Color, GammaRamp, Rect as SurfaceRect, Surface};
use clonk_gui::Rect as GuiRect;
use std::path::PathBuf;

/// `C4StartupEditBorderColor` (`src/C4Startup.h:31`).
const STARTUP_EDIT_BORDER_COLOR: u32 = 0x00a4_947a;

/// `C4PlayerInfoCore::PlayerColors`, in `0x00RRGGBB` form.
pub const PLAYER_COLORS: [u32; 12] = [
    0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xc48444, 0x784830, 0xa04400, 0xf08050, 0x848484,
    0xffffff, 0x0094f8, 0xbc00c0,
];

/// Native `C4MaxControlSet`: Keyboard1 through Gamepad4.
pub const PLAYER_CONTROL_SET_COUNT: i32 = 8;

/// `C4MaxName` bytes, excluding the terminating NUL.
pub const PLAYER_NAME_MAX_BYTES: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPropertiesMode {
    New,
    Edit { index: usize },
}

/// Requested mutation for one image entry in the player group.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerImageUpdate {
    /// Preserve the existing group entry byte-for-byte.
    Keep,
    /// Replace (or create) the group entry with this image.
    Replace(ImageData),
    /// Remove the group entry if it exists.
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerPropertiesAction {
    Submit,
    Cancel,
    ChoosePicture,
    PortraitLocationChanged { index: usize, path: PathBuf },
    ApplyPicture(PortraitSelCommit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerColorComponent {
    Red,
    Green,
    Blue,
}

/// Public controls make keyboard focus and pointer tests deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPropertiesControl {
    Name,
    Picture,
    ColorPrevious,
    ColorNext,
    Red,
    Green,
    Blue,
    ControlPrevious,
    ControlNext,
    Mouse,
    ClassicMovement,
    JumpAndRunMovement,
    Ok,
    Cancel,
}

impl PlayerPropertiesControl {
    const ORDER: [Self; 14] = [
        Self::Name,
        Self::Picture,
        Self::ColorPrevious,
        Self::ColorNext,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::ControlPrevious,
        Self::ControlNext,
        Self::Mouse,
        Self::ClassicMovement,
        Self::JumpAndRunMovement,
        Self::Ok,
        Self::Cancel,
    ];
}

/// Responsive geometry for the centered 365x400 property paper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerPropertiesLayout {
    pub paper: IntRect,
    pub title: IntRect,
    pub name: IntRect,
    pub portrait: IntRect,
    pub picture: IntRect,
    pub color_swatch: IntRect,
    pub color_previous: IntRect,
    pub color_next: IntRect,
    pub rgb_sliders: [IntRect; 3],
    pub control_previous: IntRect,
    pub control_next: IntRect,
    pub control_preview: IntRect,
    pub mouse: IntRect,
    pub classic_movement: IntRect,
    pub jump_and_run_movement: IntRect,
    pub ok: IntRect,
    pub cancel: IntRect,
    pub close: IntRect,
}

impl PlayerPropertiesLayout {
    pub fn for_size(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let scale = (width as f32 / 365.0)
            .min(height as f32 / 400.0)
            .min(1.0)
            .max(0.01);
        let paper_w = (365.0 * scale).round() as i32;
        let paper_h = (400.0 * scale).round() as i32;
        let paper = IntRect {
            x: (width - paper_w) / 2,
            y: (height - paper_h) / 2,
            w: paper_w,
            h: paper_h,
        };
        let rect = |x: i32, y: i32, w: i32, h: i32| IntRect {
            x: paper.x + (x as f32 * scale).round() as i32,
            y: paper.y + (y as f32 * scale).round() as i32,
            w: (w as f32 * scale).round().max(1.0) as i32,
            h: (h as f32 * scale).round().max(1.0) as i32,
        };
        Self {
            paper,
            title: rect(45, 20, 275, 28),
            name: rect(62, 55, 241, 28),
            portrait: rect(37, 98, 104, 104),
            picture: rect(38, 207, 102, 27),
            color_swatch: rect(181, 99, 82, 38),
            color_previous: rect(151, 103, 25, 30),
            color_next: rect(268, 103, 25, 30),
            rgb_sliders: [
                rect(166, 151, 132, 19),
                rect(166, 178, 132, 19),
                rect(166, 205, 132, 19),
            ],
            control_previous: rect(38, 253, 25, 34),
            control_next: rect(302, 253, 25, 34),
            control_preview: rect(68, 245, 228, 52),
            mouse: rect(126, 300, 113, 25),
            classic_movement: rect(62, 330, 112, 25),
            jump_and_run_movement: rect(190, 330, 125, 25),
            ok: rect(88, 358, 88, 32),
            cancel: rect(189, 358, 88, 32),
            close: rect(316, 14, 28, 28),
        }
    }

    fn rect_for(self, control: PlayerPropertiesControl) -> IntRect {
        match control {
            PlayerPropertiesControl::Name => self.name,
            PlayerPropertiesControl::Picture => self.picture,
            PlayerPropertiesControl::ColorPrevious => self.color_previous,
            PlayerPropertiesControl::ColorNext => self.color_next,
            PlayerPropertiesControl::Red => self.rgb_sliders[0],
            PlayerPropertiesControl::Green => self.rgb_sliders[1],
            PlayerPropertiesControl::Blue => self.rgb_sliders[2],
            PlayerPropertiesControl::ControlPrevious => self.control_previous,
            PlayerPropertiesControl::ControlNext => self.control_next,
            PlayerPropertiesControl::Mouse => self.mouse,
            PlayerPropertiesControl::ClassicMovement => self.classic_movement,
            PlayerPropertiesControl::JumpAndRunMovement => self.jump_and_run_movement,
            PlayerPropertiesControl::Ok => self.ok,
            PlayerPropertiesControl::Cancel => self.cancel,
        }
    }

    /// Autosized movement-label bounds in `[Classic, Jump'n'Run]` order.
    /// Native labels start six pixels above the matching icon's bottom and
    /// continue for the full BookSmallFont line height.
    pub fn movement_label_rects(self, book_small: &ClonkFont) -> [IntRect; 2] {
        [
            movement_label_rect(self.classic_movement, "Classic", book_small),
            movement_label_rect(self.jump_and_run_movement, "Jump'n'Run", book_small),
        ]
    }
}

fn movement_label_rect(button: IntRect, text: &str, font: &ClonkFont) -> IntRect {
    let (w, h) = font.measure(text, true);
    IntRect {
        x: button.x + button.w / 2 - w / 2,
        y: button.y + button.h - 6,
        w,
        h,
    }
}

/// Images used by [`PlayerPropertiesScreen`].
pub struct PlayerPropertiesAssets {
    /// `StartupPlrPropBG.png`.
    pub background: ImageData,
    /// `GUICaption.png`.
    pub caption: ImageData,
    /// `GUIButton.png`.
    pub button: ImageData,
    /// `GUIButtonDown.png`.
    pub button_down: ImageData,
    /// `GUIButtonHighlight.png`.
    pub button_highlight: ImageData,
    /// `StartupPlrCtrlType.png`; absence leaves a textual control preview.
    pub control_types: Option<ImageData>,
}

pub struct PlayerPropertiesController {
    width: i32,
    height: i32,
    mode: PlayerPropertiesMode,
    player: PlayerFile,
    comment: String,
    portrait_preview: Option<ImageData>,
    big_icon_preview: Option<ImageData>,
    portrait_update: PlayerImageUpdate,
    big_icon_update: PlayerImageUpdate,
    validation_error: Option<String>,
    portrait_selector: Option<PortraitSelController>,
    focus: PlayerPropertiesControl,
    pointer_position: Option<GuiPoint>,
    hovered: Option<PlayerPropertiesControl>,
    pointer_pressed: Option<PlayerPropertiesControl>,
}

impl PlayerPropertiesController {
    /// Constructs the editor for a not-yet-created player. Initial images are
    /// replacements because a new group has no entries to preserve.
    pub fn new(
        mut player: PlayerFile,
        comment: impl Into<String>,
        portrait: Option<ImageData>,
        big_icon: Option<ImageData>,
    ) -> Self {
        normalize_initial_player(&mut player);
        let portrait_update = portrait
            .clone()
            .map_or(PlayerImageUpdate::Clear, PlayerImageUpdate::Replace);
        let big_icon_update = big_icon
            .clone()
            .map_or(PlayerImageUpdate::Clear, PlayerImageUpdate::Replace);
        Self::from_parts(
            PlayerPropertiesMode::New,
            player,
            comment.into(),
            portrait,
            big_icon,
            portrait_update,
            big_icon_update,
        )
    }

    /// Descriptive alias for [`Self::new`].
    pub fn new_player(
        player: PlayerFile,
        comment: impl Into<String>,
        portrait: Option<ImageData>,
        big_icon: Option<ImageData>,
    ) -> Self {
        Self::new(player, comment, portrait, big_icon)
    }

    /// Constructs the editor for an existing list entry. Initial images are
    /// previews only and remain `Keep` until explicitly replaced or cleared.
    pub fn edit(
        index: usize,
        mut player: PlayerFile,
        comment: impl Into<String>,
        portrait: Option<ImageData>,
        big_icon: Option<ImageData>,
    ) -> Self {
        normalize_initial_player(&mut player);
        Self::from_parts(
            PlayerPropertiesMode::Edit { index },
            player,
            comment.into(),
            portrait,
            big_icon,
            PlayerImageUpdate::Keep,
            PlayerImageUpdate::Keep,
        )
    }

    /// Descriptive alias for [`Self::edit`].
    pub fn edit_player(
        index: usize,
        player: PlayerFile,
        comment: impl Into<String>,
        portrait: Option<ImageData>,
        big_icon: Option<ImageData>,
    ) -> Self {
        Self::edit(index, player, comment, portrait, big_icon)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        mode: PlayerPropertiesMode,
        mut player: PlayerFile,
        comment: String,
        portrait_preview: Option<ImageData>,
        big_icon_preview: Option<ImageData>,
        portrait_update: PlayerImageUpdate,
        big_icon_update: PlayerImageUpdate,
    ) -> Self {
        player.name = truncate_c4_name(&player.name);
        Self {
            width: 365,
            height: 400,
            mode,
            player,
            comment,
            portrait_preview,
            big_icon_preview,
            portrait_update,
            big_icon_update,
            validation_error: None,
            portrait_selector: None,
            // The C++ name edit is the initially focused control.
            focus: PlayerPropertiesControl::Name,
            pointer_position: None,
            hovered: None,
            pointer_pressed: None,
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        if let Some(selector) = self.portrait_selector.as_mut() {
            selector.resize(self.width, self.height);
        }
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_control(point));
    }

    pub const fn mode(&self) -> PlayerPropertiesMode {
        self.mode
    }

    pub const fn player(&self) -> &PlayerFile {
        &self.player
    }

    /// Direct mutation access for application-owned fields such as retained
    /// score and crew data. Prefer the semantic setters for displayed fields.
    pub fn player_mut(&mut self) -> &mut PlayerFile {
        &mut self.player
    }

    pub fn comment(&self) -> &str {
        &self.comment
    }

    pub fn set_comment(&mut self, comment: impl Into<String>) {
        self.comment = comment.into();
    }

    pub fn name(&self) -> &str {
        &self.player.name
    }

    pub fn set_name(&mut self, name: impl AsRef<str>) {
        self.player.name = truncate_c4_name(name.as_ref());
        self.validation_error = None;
    }

    pub fn delete_name_char(&mut self) -> bool {
        if self.portrait_selector.is_some() || self.focus != PlayerPropertiesControl::Name {
            return false;
        }
        let changed = self.player.name.pop().is_some();
        if changed {
            self.validation_error = None;
        }
        changed
    }

    pub const fn focused_control(&self) -> PlayerPropertiesControl {
        self.focus
    }

    pub fn set_focused_control(&mut self, focus: PlayerPropertiesControl) {
        self.focus = focus;
    }

    pub fn validation_error(&self) -> Option<&str> {
        self.validation_error.as_deref()
    }

    pub fn set_validation_error(&mut self, error: Option<String>) {
        self.validation_error = error;
    }

    pub fn clear_validation_error(&mut self) {
        self.validation_error = None;
    }

    pub fn portrait_preview(&self) -> Option<&ImageData> {
        self.portrait_preview.as_ref()
    }

    pub fn big_icon_preview(&self) -> Option<&ImageData> {
        self.big_icon_preview.as_ref()
    }

    pub const fn portrait_update(&self) -> &PlayerImageUpdate {
        &self.portrait_update
    }

    pub const fn big_icon_update(&self) -> &PlayerImageUpdate {
        &self.big_icon_update
    }

    pub fn replace_portrait(&mut self, image: ImageData) {
        self.portrait_preview = Some(image.clone());
        self.portrait_update = PlayerImageUpdate::Replace(image);
    }

    pub fn clear_portrait(&mut self) {
        self.portrait_preview = None;
        self.portrait_update = PlayerImageUpdate::Clear;
    }

    pub fn keep_portrait(&mut self) {
        self.portrait_update = PlayerImageUpdate::Keep;
    }

    pub fn replace_big_icon(&mut self, image: ImageData) {
        self.big_icon_preview = Some(image.clone());
        self.big_icon_update = PlayerImageUpdate::Replace(image);
    }

    pub fn clear_big_icon(&mut self) {
        self.big_icon_preview = None;
        self.big_icon_update = PlayerImageUpdate::Clear;
    }

    pub fn keep_big_icon(&mut self) {
        self.big_icon_update = PlayerImageUpdate::Keep;
    }

    pub fn replace_images(&mut self, portrait: ImageData, big_icon: ImageData) {
        self.replace_portrait(portrait);
        self.replace_big_icon(big_icon);
    }

    pub fn clear_images(&mut self) {
        self.clear_portrait();
        self.clear_big_icon();
    }

    /// Applies one accepted portrait-selector choice without disturbing an
    /// unchecked channel. `None` for a checked channel is the working
    /// `<none>` item and therefore records an explicit clear.
    pub fn apply_picture_selection(
        &mut self,
        portrait: Option<ImageData>,
        big_icon: Option<ImageData>,
        set_picture: bool,
        set_big_icon: bool,
    ) {
        if set_picture {
            match portrait {
                Some(image) => self.replace_portrait(image),
                None => self.clear_portrait(),
            }
        }
        if set_big_icon {
            match big_icon {
                Some(image) => self.replace_big_icon(image),
                None => self.clear_big_icon(),
            }
        }
    }

    pub fn open_portrait_selector(
        &mut self,
        locations: Vec<PortraitLocation>,
        current_location: usize,
        entries: Vec<PortraitFileEntry>,
    ) {
        let mut selector = PortraitSelController::new(
            locations,
            current_location,
            entries,
            self.portrait_preview.is_some(),
            self.big_icon_preview.is_some(),
        );
        selector.resize(self.width, self.height);
        self.pointer_position = None;
        self.hovered = None;
        self.pointer_pressed = None;
        self.portrait_selector = Some(selector);
    }

    pub const fn portrait_selector(&self) -> Option<&PortraitSelController> {
        self.portrait_selector.as_ref()
    }

    pub fn portrait_selector_mut(&mut self) -> Option<&mut PortraitSelController> {
        self.portrait_selector.as_mut()
    }

    pub fn replace_portrait_location_entries(
        &mut self,
        location_index: usize,
        entries: Vec<PortraitFileEntry>,
    ) -> bool {
        self.portrait_selector
            .as_mut()
            .is_some_and(|selector| selector.replace_location_entries(location_index, entries))
    }

    pub fn fail_portrait_location_entries(
        &mut self,
        location_index: usize,
        error: impl Into<String>,
    ) {
        if let Some(selector) = self.portrait_selector.as_mut() {
            selector.fail_location_entries(location_index, error);
        }
    }

    pub fn advance_portrait_selector_idle(&mut self) -> Option<PortraitThumbnailRequest> {
        self.portrait_selector.as_mut()?.advance_idle()
    }

    pub fn complete_portrait_thumbnail(
        &mut self,
        request: &PortraitThumbnailRequest,
        thumbnail: Result<ImageData, String>,
    ) -> bool {
        self.portrait_selector
            .as_mut()
            .is_some_and(|selector| selector.complete_thumbnail(request, thumbnail))
    }

    pub fn close_portrait_selector(&mut self) {
        self.portrait_selector = None;
    }

    pub fn set_portrait_selector_error(&mut self, error: impl Into<String>) {
        if let Some(selector) = self.portrait_selector.as_mut() {
            selector.set_validation_error(error);
        }
    }

    pub fn color(&self) -> u32 {
        self.player.pref_color_dw & 0x00ff_ffff
    }

    pub fn set_color(&mut self, color: u32) {
        let color = color & 0x00ff_ffff;
        self.player.pref_color_dw = if color == 0 { 1 } else { color };
    }

    pub fn cycle_color(&mut self, backwards: bool) {
        let delta = if backwards { -1 } else { 1 };
        let index = (self
            .player
            .pref_color
            .rem_euclid(PLAYER_COLORS.len() as i32)
            + delta)
            .rem_euclid(PLAYER_COLORS.len() as i32);
        self.player.pref_color = index;
        self.player.pref_color_dw = PLAYER_COLORS[index as usize];
    }

    pub fn set_color_component(&mut self, component: PlayerColorComponent, value: u8) {
        let shift = match component {
            PlayerColorComponent::Red => 16,
            PlayerColorComponent::Green => 8,
            PlayerColorComponent::Blue => 0,
        };
        let mask = !(0xff_u32 << shift);
        let color = (self.color() & mask) | (u32::from(value) << shift);
        self.set_color(color);
    }

    pub fn color_component(&self, component: PlayerColorComponent) -> u8 {
        let shift = match component {
            PlayerColorComponent::Red => 16,
            PlayerColorComponent::Green => 8,
            PlayerColorComponent::Blue => 0,
        };
        ((self.color() >> shift) & 0xff) as u8
    }

    pub fn cycle_control(&mut self, backwards: bool) {
        let delta = if backwards { -1 } else { 1 };
        self.player.pref_control = (self
            .player
            .pref_control
            .rem_euclid(PLAYER_CONTROL_SET_COUNT)
            + delta)
            .rem_euclid(PLAYER_CONTROL_SET_COUNT);
    }

    pub fn set_control(&mut self, control: i32) {
        self.player.pref_control = control.rem_euclid(PLAYER_CONTROL_SET_COUNT);
    }

    pub fn toggle_mouse(&mut self) {
        self.player.pref_mouse = !self.player.pref_mouse;
    }

    /// Movement selection updates both persisted flags, as the property
    /// dialog mirrors AutoContextMenu from PrefControlStyle.
    pub fn set_movement_style(&mut self, jump_and_run: bool) {
        self.player.pref_control_style = jump_and_run;
        self.player.pref_auto_context_menu = jump_and_run;
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.portrait_selector
            .as_ref()
            .and_then(PortraitSelController::pointer_position)
            .or(self.pointer_position)
    }

    /// Returns exactly the tooltips assigned by the native player-properties
    /// dialog. Untipped edits, previews, swatches and OK/Cancel controls do
    /// not inherit descriptions from unrelated siblings.
    pub fn tooltip_at(&self, point: GuiPoint, book_small: &ClonkFont) -> Option<StartupTooltip> {
        if self.portrait_selector.is_some() {
            return None;
        }
        let layout = self.layout();
        if contains(layout.control_preview, point) {
            return Some(StartupTooltip::resource("IDS_DLGTIP_PLAYERCONTROL"));
        }
        if let Some(control) = self.hit_control(point) {
            let key = match control {
                PlayerPropertiesControl::ColorPrevious | PlayerPropertiesControl::ColorNext => {
                    "IDS_DLGTIP_PLAYERCOLORS"
                }
                PlayerPropertiesControl::Red
                | PlayerPropertiesControl::Green
                | PlayerPropertiesControl::Blue => "IDS_DLGTIP_PLAYERCOLORSTGB",
                PlayerPropertiesControl::ControlPrevious | PlayerPropertiesControl::ControlNext => {
                    "IDS_DLGTIP_PLAYERCONTROL"
                }
                PlayerPropertiesControl::Mouse => "IDS_DLGTIP_PLAYERCONTROLMOUSE",
                PlayerPropertiesControl::Picture => "IDS_DESC_SELECTAPICTUREANDORLOBBYI",
                PlayerPropertiesControl::JumpAndRunMovement => "IDS_DLGTIP_JUMPANDRUN",
                PlayerPropertiesControl::ClassicMovement => "IDS_DLGTIP_CLASSIC",
                PlayerPropertiesControl::Name
                | PlayerPropertiesControl::Ok
                | PlayerPropertiesControl::Cancel => return None,
            };
            return Some(StartupTooltip::resource(key));
        }
        let [classic, jump_and_run] = layout.movement_label_rects(book_small);
        if contains(classic, point) {
            return Some(StartupTooltip::resource("IDS_DLGTIP_CLASSIC"));
        }
        contains(jump_and_run, point).then(|| StartupTooltip::resource("IDS_DLGTIP_JUMPANDRUN"))
    }

    pub fn tooltip(&self, book_small: &ClonkFont) -> Option<StartupTooltip> {
        self.tooltip_at(self.pointer_position?, book_small)
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        if let Some(selector) = self.portrait_selector.as_mut() {
            match position {
                Some(point) => {
                    selector.handle_pointer_move(point);
                }
                None => selector.pointer_left(),
            }
            return;
        }
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_control(point));
        if position.is_none() {
            self.pointer_pressed = None;
        }
    }

    pub fn pointer_left(&mut self) {
        if let Some(selector) = self.portrait_selector.as_mut() {
            selector.pointer_left();
            return;
        }
        self.set_pointer_position(None);
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_move(position);
            return self.finish_portrait_selector_actions(actions);
        }
        self.pointer_position = Some(position);
        self.hovered = self.hit_control(position);
        if self.pointer_pressed.is_some_and(is_slider) {
            self.update_slider_from_pointer(self.pointer_pressed.unwrap(), position);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_down(position);
            return self.finish_portrait_selector_actions(actions);
        }
        self.pointer_position = Some(position);
        self.hovered = self.hit_control(position);
        if contains(self.layout().close, position) {
            self.pointer_pressed = None;
            return vec![PlayerPropertiesAction::Cancel];
        }
        self.pointer_pressed = self.hovered;
        if let Some(control) = self.hovered {
            self.focus = control;
            if is_slider(control) {
                self.update_slider_from_pointer(control, position);
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_up(position);
            return self.finish_portrait_selector_actions(actions);
        }
        self.pointer_position = Some(position);
        self.hovered = self.hit_control(position);
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if is_slider(pressed) || self.hovered != Some(pressed) {
            return Vec::new();
        }
        self.activate(pressed)
    }

    /// Adds printable window text while the name edit owns focus. The limit
    /// is measured in native C4 bytes, not UTF-8 storage bytes.
    pub fn handle_text_input(&mut self, text: &str) -> Vec<PlayerPropertiesAction> {
        if self.portrait_selector.is_some() {
            return Vec::new();
        }
        if self.focus != PlayerPropertiesControl::Name {
            return Vec::new();
        }
        let mut bytes = clonk_script::c4_string_bytes(&self.player.name);
        for character in text.chars().filter(|character| !character.is_control()) {
            let encoded = clonk_script::c4_string_bytes(&character.to_string());
            if bytes.len() + encoded.len() > PLAYER_NAME_MAX_BYTES {
                break;
            }
            bytes.extend(encoded);
        }
        self.player.name = clonk_script::c4_string_from_bytes(&bytes);
        self.validation_error = None;
        Vec::new()
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_key_down(key);
            return self.finish_portrait_selector_actions(actions);
        }
        match key {
            KeyCode::Escape => vec![PlayerPropertiesAction::Cancel],
            KeyCode::Enter => vec![PlayerPropertiesAction::Submit],
            KeyCode::Tab | KeyCode::Down => {
                self.move_focus(false);
                Vec::new()
            }
            KeyCode::Up => {
                self.move_focus(true);
                Vec::new()
            }
            KeyCode::Left => {
                self.adjust_focused(true);
                Vec::new()
            }
            KeyCode::Right => {
                self.adjust_focused(false);
                Vec::new()
            }
            KeyCode::Space => self.activate(self.focus),
            KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_key_up(key);
            return self.finish_portrait_selector_actions(actions);
        }
        Vec::new()
    }

    pub fn handle_wheel(&mut self, native_delta: i32) -> bool {
        self.portrait_selector
            .as_mut()
            .is_some_and(|selector| selector.handle_wheel(native_delta))
    }

    fn layout(&self) -> PlayerPropertiesLayout {
        PlayerPropertiesLayout::for_size(self.width, self.height)
    }

    fn hit_control(&self, point: GuiPoint) -> Option<PlayerPropertiesControl> {
        let layout = self.layout();
        PlayerPropertiesControl::ORDER
            .into_iter()
            .find(|control| contains(layout.rect_for(*control), point))
    }

    fn move_focus(&mut self, backwards: bool) {
        let position = PlayerPropertiesControl::ORDER
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let count = PlayerPropertiesControl::ORDER.len();
        let next = if backwards {
            (position + count - 1) % count
        } else {
            (position + 1) % count
        };
        self.focus = PlayerPropertiesControl::ORDER[next];
    }

    fn adjust_focused(&mut self, backwards: bool) {
        match self.focus {
            PlayerPropertiesControl::ColorPrevious | PlayerPropertiesControl::ColorNext => {
                self.cycle_color(backwards)
            }
            PlayerPropertiesControl::Red => {
                self.step_component(PlayerColorComponent::Red, backwards)
            }
            PlayerPropertiesControl::Green => {
                self.step_component(PlayerColorComponent::Green, backwards)
            }
            PlayerPropertiesControl::Blue => {
                self.step_component(PlayerColorComponent::Blue, backwards)
            }
            PlayerPropertiesControl::ControlPrevious | PlayerPropertiesControl::ControlNext => {
                self.cycle_control(backwards)
            }
            _ => {}
        }
    }

    fn step_component(&mut self, component: PlayerColorComponent, backwards: bool) {
        let old = self.color_component(component);
        let new = if backwards {
            old.saturating_sub(1)
        } else {
            old.saturating_add(1)
        };
        self.set_color_component(component, new);
    }

    fn update_slider_from_pointer(&mut self, control: PlayerPropertiesControl, point: GuiPoint) {
        let (component, rect) = match control {
            PlayerPropertiesControl::Red => {
                (PlayerColorComponent::Red, self.layout().rgb_sliders[0])
            }
            PlayerPropertiesControl::Green => {
                (PlayerColorComponent::Green, self.layout().rgb_sliders[1])
            }
            PlayerPropertiesControl::Blue => {
                (PlayerColorComponent::Blue, self.layout().rgb_sliders[2])
            }
            _ => return,
        };
        let relative = (point.x.round() as i32 - rect.x).clamp(0, rect.w.saturating_sub(1));
        let divisor = rect.w.saturating_sub(1).max(1);
        let value = (relative * 255 / divisor) as u8;
        self.set_color_component(component, value);
    }

    fn activate(&mut self, control: PlayerPropertiesControl) -> Vec<PlayerPropertiesAction> {
        match control {
            PlayerPropertiesControl::ColorPrevious => self.cycle_color(true),
            PlayerPropertiesControl::ColorNext => self.cycle_color(false),
            PlayerPropertiesControl::ControlPrevious => self.cycle_control(true),
            PlayerPropertiesControl::ControlNext => self.cycle_control(false),
            PlayerPropertiesControl::Mouse => self.toggle_mouse(),
            PlayerPropertiesControl::ClassicMovement => self.set_movement_style(false),
            PlayerPropertiesControl::JumpAndRunMovement => self.set_movement_style(true),
            PlayerPropertiesControl::Picture => return vec![PlayerPropertiesAction::ChoosePicture],
            PlayerPropertiesControl::Ok => return vec![PlayerPropertiesAction::Submit],
            PlayerPropertiesControl::Cancel => return vec![PlayerPropertiesAction::Cancel],
            PlayerPropertiesControl::Name
            | PlayerPropertiesControl::Red
            | PlayerPropertiesControl::Green
            | PlayerPropertiesControl::Blue => {}
        }
        Vec::new()
    }

    fn finish_portrait_selector_actions(
        &mut self,
        actions: Vec<PortraitSelAction>,
    ) -> Vec<PlayerPropertiesAction> {
        let mut outer = Vec::new();
        for action in actions {
            match action {
                PortraitSelAction::Cancel => self.close_portrait_selector(),
                PortraitSelAction::ChangeLocation { index, path } => {
                    outer.push(PlayerPropertiesAction::PortraitLocationChanged { index, path });
                }
                PortraitSelAction::Accept(commit) => {
                    outer.push(PlayerPropertiesAction::ApplyPicture(commit));
                }
            }
        }
        outer
    }
}

fn normalize_initial_player(player: &mut PlayerFile) {
    player.pref_color = player.pref_color.clamp(0, PLAYER_COLORS.len() as i32 - 1);
    player.pref_color_dw &= 0x00ff_ffff;
    if player.pref_color_dw == 0 {
        player.pref_color_dw = 0xff;
    }
    player.pref_control = player.pref_control.clamp(0, PLAYER_CONTROL_SET_COUNT - 1);
}

fn truncate_c4_name(value: &str) -> String {
    let mut result = String::new();
    let mut byte_len = 0;
    for character in value.chars() {
        let size = clonk_script::c4_string_bytes(&character.to_string()).len();
        if byte_len + size > PLAYER_NAME_MAX_BYTES {
            break;
        }
        result.push(character);
        byte_len += size;
    }
    result
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn is_slider(control: PlayerPropertiesControl) -> bool {
    matches!(
        control,
        PlayerPropertiesControl::Red
            | PlayerPropertiesControl::Green
            | PlayerPropertiesControl::Blue
    )
}

/// Draws the property paper and all live controls. The startup selection
/// screen may be drawn first; transparent paper pixels intentionally preserve
/// that backdrop.
pub struct PlayerPropertiesScreen;

impl PlayerPropertiesScreen {
    pub fn render(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        fonts: &ClonkFontSet,
        controller: &PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        let layout =
            PlayerPropertiesLayout::for_size(surface.width() as i32, surface.height() as i32);
        crate::draw_image_bilinear(surface, &gui_rect(layout.paper), &assets.background, gamma);

        let skin = ClassicGuiSkin::new(
            &assets.caption,
            &assets.button,
            &assets.button_down,
            Some(&assets.button_highlight),
        );
        let highlighted =
            |control| controller.focus == control || controller.hovered == Some(control);
        let pressed = |control| controller.pointer_pressed == Some(control);

        fonts.caption.draw_with_gamma(
            surface,
            layout.title.x + layout.title.w / 2,
            layout.title.y,
            match controller.mode {
                PlayerPropertiesMode::New => "New Player",
                PlayerPropertiesMode::Edit { .. } => "Player Properties",
            },
            [0x40, 0x20, 0x08, 0xff],
            TextAlign::Center,
            true,
            gamma,
        );

        skin.draw_caption(
            surface,
            layout.name,
            &controller.player.name,
            &fonts.text,
            [0xff, 0xff, 0xff, 0xff],
            TextAlign::Left,
            gamma,
        );
        draw_name_edit_frames(surface, layout.name, gamma);

        if let Some(portrait) = controller.portrait_preview.as_ref() {
            crate::draw_image_bilinear(surface, &gui_rect(layout.portrait), portrait, gamma);
        } else if let Some(big_icon) = controller.big_icon_preview.as_ref() {
            crate::draw_image_bilinear(surface, &gui_rect(layout.portrait), big_icon, gamma);
        } else {
            fill(surface, layout.portrait, Color::new(40, 30, 20, 160));
        }
        skin.draw_button(
            surface,
            layout.picture,
            "Picture",
            fonts,
            ClassicButtonState {
                pressed: pressed(PlayerPropertiesControl::Picture),
                highlighted: highlighted(PlayerPropertiesControl::Picture),
            },
            gamma,
        );

        let color = controller.color();
        fill(
            surface,
            layout.color_swatch,
            Color::opaque(
                ((color >> 16) & 0xff) as u8,
                ((color >> 8) & 0xff) as u8,
                (color & 0xff) as u8,
            ),
        );
        draw_outline(surface, layout.color_swatch, Color::opaque(80, 40, 12));
        draw_small_button(
            surface,
            &skin,
            fonts,
            layout.color_previous,
            "<",
            PlayerPropertiesControl::ColorPrevious,
            controller,
            gamma,
        );
        draw_small_button(
            surface,
            &skin,
            fonts,
            layout.color_next,
            ">",
            PlayerPropertiesControl::ColorNext,
            controller,
            gamma,
        );

        for (index, (component, label, tint)) in [
            (PlayerColorComponent::Red, "R", Color::opaque(160, 25, 25)),
            (PlayerColorComponent::Green, "G", Color::opaque(25, 150, 25)),
            (PlayerColorComponent::Blue, "B", Color::opaque(25, 55, 170)),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = layout.rgb_sliders[index];
            fill(surface, rect, Color::new(30, 20, 10, 180));
            let value = i32::from(controller.color_component(component));
            let width = ((rect.w - 4).max(1) * value / 255).max(1);
            fill(
                surface,
                IntRect {
                    x: rect.x + 2,
                    y: rect.y + 2,
                    w: width,
                    h: (rect.h - 4).max(1),
                },
                tint,
            );
            fonts.mini.draw_with_gamma(
                surface,
                rect.x - 8,
                rect.y + (rect.h - fonts.mini.line_height) / 2,
                label,
                [0x40, 0x20, 0x08, 0xff],
                TextAlign::Right,
                false,
                gamma,
            );
        }

        if let Some(control_types) = assets.control_types.as_ref() {
            crate::draw_image_bilinear(
                surface,
                &gui_rect(layout.control_preview),
                control_types,
                gamma,
            );
        } else {
            skin.draw_caption(
                surface,
                layout.control_preview,
                &format!("Control {}", controller.player.pref_control + 1),
                &fonts.text,
                [0xff, 0xff, 0xff, 0xff],
                TextAlign::Center,
                gamma,
            );
        }
        draw_small_button(
            surface,
            &skin,
            fonts,
            layout.control_previous,
            "<",
            PlayerPropertiesControl::ControlPrevious,
            controller,
            gamma,
        );
        draw_small_button(
            surface,
            &skin,
            fonts,
            layout.control_next,
            ">",
            PlayerPropertiesControl::ControlNext,
            controller,
            gamma,
        );
        draw_toggle(
            surface,
            &skin,
            fonts,
            layout.mouse,
            "Mouse",
            controller.player.pref_mouse,
            PlayerPropertiesControl::Mouse,
            controller,
            gamma,
        );
        draw_toggle(
            surface,
            &skin,
            fonts,
            layout.classic_movement,
            "Classic",
            !controller.player.pref_control_style,
            PlayerPropertiesControl::ClassicMovement,
            controller,
            gamma,
        );
        draw_toggle(
            surface,
            &skin,
            fonts,
            layout.jump_and_run_movement,
            "Jump'n'Run",
            controller.player.pref_control_style,
            PlayerPropertiesControl::JumpAndRunMovement,
            controller,
            gamma,
        );

        for (control, rect, label) in [
            (PlayerPropertiesControl::Ok, layout.ok, "OK"),
            (PlayerPropertiesControl::Cancel, layout.cancel, "Cancel"),
        ] {
            skin.draw_button(
                surface,
                rect,
                label,
                fonts,
                ClassicButtonState {
                    pressed: pressed(control),
                    highlighted: highlighted(control),
                },
                gamma,
            );
        }

        if let Some(error) = controller.validation_error.as_deref() {
            fonts.mini.draw_with_gamma(
                surface,
                layout.paper.x + layout.paper.w / 2,
                layout.ok.y - fonts.mini.line_height - 2,
                error,
                [0xff, 0x30, 0x20, 0xff],
                TextAlign::Center,
                false,
                gamma,
            );
        }

        if let Some(selector) = controller.portrait_selector.as_ref() {
            selector.render(surface, PortraitSelResources { skin, fonts }, gamma);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_small_button(
    surface: &mut Surface,
    skin: &ClassicGuiSkin<'_>,
    fonts: &ClonkFontSet,
    rect: IntRect,
    label: &str,
    control: PlayerPropertiesControl,
    controller: &PlayerPropertiesController,
    gamma: Option<&GammaRamp>,
) {
    skin.draw_button(
        surface,
        rect,
        label,
        fonts,
        ClassicButtonState {
            pressed: controller.pointer_pressed == Some(control),
            highlighted: controller.focus == control || controller.hovered == Some(control),
        },
        gamma,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_toggle(
    surface: &mut Surface,
    skin: &ClassicGuiSkin<'_>,
    fonts: &ClonkFontSet,
    rect: IntRect,
    label: &str,
    selected: bool,
    control: PlayerPropertiesControl,
    controller: &PlayerPropertiesController,
    gamma: Option<&GammaRamp>,
) {
    let label = if selected {
        format!("[x] {label}")
    } else {
        format!("[ ] {label}")
    };
    draw_small_button(
        surface, skin, fonts, rect, &label, control, controller, gamma,
    );
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
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

fn draw_name_edit_frames(surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
    if rect.w <= 1 || rect.h <= 2 {
        return;
    }
    crate::classic_gui::draw_engine_frame(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w,
        rect.y + rect.h - 1,
        STARTUP_EDIT_BORDER_COLOR,
        gamma,
    );
    crate::classic_gui::draw_engine_frame(
        surface,
        rect.x + 1,
        rect.y + 1,
        rect.x + rect.w - 1,
        rect.y + rect.h - 2,
        STARTUP_EDIT_BORDER_COLOR,
        gamma,
    );
}

fn draw_outline(surface: &mut Surface, rect: IntRect, color: Color) {
    fill(surface, IntRect { h: 1, ..rect }, color);
    fill(
        surface,
        IntRect {
            y: rect.y + rect.h - 1,
            h: 1,
            ..rect
        },
        color,
    );
    fill(surface, IntRect { w: 1, ..rect }, color);
    fill(
        surface,
        IntRect {
            x: rect.x + rect.w - 1,
            w: 1,
            ..rect
        },
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_name_edit_keeps_both_native_draw_frame_passes() {
        let mut surface = Surface::new(32, 16, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_name_edit_frames(
            &mut surface,
            IntRect {
                x: 2,
                y: 2,
                w: 20,
                h: 10,
            },
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([32, 16], Color::transparent(), &GammaRamp::identity());
        let [clonk_graphics::GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        }] = scene.commands.as_slice()
        else {
            panic!("both edit frames should coalesce into one retained line batch");
        };
        assert_eq!(vertices.len(), 16, "two four-segment DrawFrameDw passes");
        assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
        assert_eq!(*alpha_mode, clonk_graphics::GpuSolidAlphaMode::SourceOver);
    }

    fn controller() -> PlayerPropertiesController {
        PlayerPropertiesController::edit(3, PlayerFile::default(), "comment", None, None)
    }

    #[test]
    fn exact_player_colors_wrap_in_both_directions() {
        let mut state = controller();
        assert_eq!(PLAYER_COLORS.len(), 12);
        state.player.pref_color = 0;
        state.cycle_color(true);
        assert_eq!(state.player.pref_color, 11);
        assert_eq!(state.player.pref_color_dw, 0xbc00c0);
        state.cycle_color(false);
        assert_eq!(state.player.pref_color, 0);
        assert_eq!(state.player.pref_color_dw, 0x0000e8);
    }

    #[test]
    fn zero_initial_and_component_results_are_coerced_nonzero() {
        let mut player = PlayerFile {
            pref_color_dw: 0,
            ..PlayerFile::default()
        };
        let mut state = PlayerPropertiesController::new(player.clone(), "", None, None);
        assert_eq!(state.color(), 0xff);
        state.set_color_component(PlayerColorComponent::Blue, 0);
        assert_eq!(state.color(), 1);
        state.set_color(0);
        assert_eq!(state.color(), 1);

        player.pref_color_dw = 0xffff_ffff;
        state = PlayerPropertiesController::edit(0, player, "", None, None);
        assert_eq!(state.color(), 0xff_ffff);
    }

    #[test]
    fn rgb_components_update_independently() {
        let mut state = controller();
        state.set_color(0x123456);
        state.set_color_component(PlayerColorComponent::Green, 0xab);
        assert_eq!(state.color(), 0x12ab56);
        state.set_color_component(PlayerColorComponent::Red, 0xcd);
        assert_eq!(state.color(), 0xcdab56);
        state.set_color_component(PlayerColorComponent::Blue, 0xef);
        assert_eq!(state.color(), 0xcdabef);
    }

    #[test]
    fn movement_style_always_mirrors_auto_context_menu() {
        let mut state = controller();
        state.player.pref_auto_context_menu = true;
        state.set_movement_style(false);
        assert!(!state.player.pref_control_style);
        assert!(!state.player.pref_auto_context_menu);
        state.set_movement_style(true);
        assert!(state.player.pref_control_style);
        assert!(state.player.pref_auto_context_menu);
    }

    #[test]
    fn controls_wrap_over_all_eight_sets_and_mouse_toggles() {
        let mut state = controller();
        state.set_control(0);
        state.cycle_control(true);
        assert_eq!(state.player.pref_control, 7);
        state.cycle_control(false);
        assert_eq!(state.player.pref_control, 0);
        let mouse = state.player.pref_mouse;
        state.toggle_mouse();
        assert_eq!(state.player.pref_mouse, !mouse);

        let state = PlayerPropertiesController::edit(
            0,
            PlayerFile {
                pref_color: 99,
                pref_control: -9,
                ..PlayerFile::default()
            },
            "",
            None,
            None,
        );
        assert_eq!(state.player.pref_color, 11);
        assert_eq!(state.player.pref_control, 0);
    }

    #[test]
    fn name_edit_is_initially_focused_and_limited_to_thirty_c4_bytes() {
        let mut state = controller();
        assert_eq!(state.focused_control(), PlayerPropertiesControl::Name);
        state.set_name("12345678901234567890123456789");
        state.handle_text_input("éZ");
        // U+00e9 is two native bytes in this representation, so it does not
        // fit in the final byte; input stops before the following character.
        assert_eq!(clonk_script::c4_string_bytes(state.name()).len(), 29);
        assert!(!state.name().ends_with('Z'));
        state.set_name("123456789012345678901234567890more");
        assert_eq!(clonk_script::c4_string_bytes(state.name()).len(), 30);
    }

    #[test]
    fn enter_escape_and_image_intents_are_explicit() {
        let image = ImageData::new(1, 1, vec![1, 2, 3, 4]);
        let mut state =
            PlayerPropertiesController::new(PlayerFile::default(), "", Some(image.clone()), None);
        assert_eq!(state.portrait_update(), &PlayerImageUpdate::Replace(image));
        assert_eq!(state.big_icon_update(), &PlayerImageUpdate::Clear);
        assert_eq!(
            state.handle_key_down(KeyCode::Enter),
            vec![PlayerPropertiesAction::Submit]
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Escape),
            vec![PlayerPropertiesAction::Cancel]
        );
        state.clear_portrait();
        assert_eq!(state.portrait_update(), &PlayerImageUpdate::Clear);
    }

    #[test]
    fn picture_control_requests_application_owned_picker() {
        let mut state = controller();
        state.set_focused_control(PlayerPropertiesControl::Picture);
        assert_eq!(
            state.handle_key_down(KeyCode::Space),
            vec![PlayerPropertiesAction::ChoosePicture]
        );
    }

    #[test]
    fn portrait_choice_updates_only_checked_image_channels() {
        let old_portrait = ImageData::new(1, 1, vec![10, 20, 30, 255]);
        let old_icon = ImageData::new(1, 1, vec![40, 50, 60, 255]);
        let mut state = PlayerPropertiesController::edit(
            0,
            PlayerFile::default(),
            "",
            Some(old_portrait),
            Some(old_icon.clone()),
        );

        state.apply_picture_selection(None, None, true, false);
        assert_eq!(state.portrait_update(), &PlayerImageUpdate::Clear);
        assert_eq!(state.big_icon_update(), &PlayerImageUpdate::Keep);
        assert_eq!(state.big_icon_preview(), Some(&old_icon));

        let replacement = ImageData::new(1, 1, vec![70, 80, 90, 255]);
        state.apply_picture_selection(None, Some(replacement.clone()), false, true);
        assert_eq!(state.portrait_update(), &PlayerImageUpdate::Clear);
        assert_eq!(
            state.big_icon_update(),
            &PlayerImageUpdate::Replace(replacement)
        );
    }

    #[test]
    fn open_portrait_selector_keeps_text_backspace_and_pointer_out_of_parent() {
        let mut state = controller();
        state.set_name("Parent");
        let original_color = state.player().pref_color_dw;
        state.open_portrait_selector(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
        );

        assert!(state.handle_text_input("X").is_empty());
        assert!(!state.delete_name_char());
        let color_next = state.layout().color_next;
        let point = GuiPoint::new(
            (color_next.x + color_next.w / 2) as f32,
            (color_next.y + color_next.h / 2) as f32,
        );
        assert!(state.handle_pointer_down(point).is_empty());
        assert!(state.handle_pointer_up(point).is_empty());
        assert_eq!(state.name(), "Parent");
        assert_eq!(state.player().pref_color_dw, original_color);
    }

    #[test]
    fn tooltip_targets_match_native_player_properties_assignments() {
        let mut state = controller();
        state.resize(800, 600);
        let layout = PlayerPropertiesLayout::for_size(800, 600);
        let ttf =
            std::fs::read(crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"))
                .expect("read Endeavour.ttf");
        let book_small = crate::startup_options_dlg::build_book_fonts(&ttf)
            .expect("build book fonts")
            .book_small;
        let center = |rect: IntRect| {
            GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
        };
        for (rect, key) in [
            (layout.color_previous, "IDS_DLGTIP_PLAYERCOLORS"),
            (layout.color_next, "IDS_DLGTIP_PLAYERCOLORS"),
            (layout.rgb_sliders[0], "IDS_DLGTIP_PLAYERCOLORSTGB"),
            (layout.rgb_sliders[1], "IDS_DLGTIP_PLAYERCOLORSTGB"),
            (layout.rgb_sliders[2], "IDS_DLGTIP_PLAYERCOLORSTGB"),
            (layout.control_previous, "IDS_DLGTIP_PLAYERCONTROL"),
            (layout.control_preview, "IDS_DLGTIP_PLAYERCONTROL"),
            (layout.control_next, "IDS_DLGTIP_PLAYERCONTROL"),
            (layout.mouse, "IDS_DLGTIP_PLAYERCONTROLMOUSE"),
            (layout.picture, "IDS_DESC_SELECTAPICTUREANDORLOBBYI"),
            (layout.jump_and_run_movement, "IDS_DLGTIP_JUMPANDRUN"),
            (layout.classic_movement, "IDS_DLGTIP_CLASSIC"),
        ] {
            assert_eq!(
                state.tooltip_at(center(rect), &book_small),
                Some(StartupTooltip::resource(key))
            );
        }
        for rect in [
            layout.name,
            layout.color_swatch,
            layout.ok,
            layout.cancel,
            layout.close,
        ] {
            assert_eq!(state.tooltip_at(center(rect), &book_small), None);
        }

        let [classic_label, jump_label] = layout.movement_label_rects(&book_small);
        for (label, button, key) in [
            (classic_label, layout.classic_movement, "IDS_DLGTIP_CLASSIC"),
            (
                jump_label,
                layout.jump_and_run_movement,
                "IDS_DLGTIP_JUMPANDRUN",
            ),
        ] {
            assert!(label.y + label.h > button.y + button.h);
            let label_only = GuiPoint::new(
                (label.x + label.w / 2) as f32,
                (button.y + button.h + 1) as f32,
            );
            assert_eq!(
                state.tooltip_at(label_only, &book_small),
                Some(StartupTooltip::resource(key))
            );
        }

        for (label, blocker) in [(classic_label, layout.ok), (jump_label, layout.cancel)] {
            let overlap = IntRect {
                x: label.x.max(blocker.x),
                y: label.y.max(blocker.y),
                w: (label.x + label.w).min(blocker.x + blocker.w) - label.x.max(blocker.x),
                h: (label.y + label.h).min(blocker.y + blocker.h) - label.y.max(blocker.y),
            };
            assert!(overlap.w > 0 && overlap.h > 0);
            assert_eq!(
                state.tooltip_at(center(overlap), &book_small),
                None,
                "later OK/Cancel controls occlude the movement label"
            );
        }
    }
}
