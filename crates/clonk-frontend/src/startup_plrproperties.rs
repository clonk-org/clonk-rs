//! Controller and classic renderer for the startup player-properties dialog.
//!
//! Persistence deliberately stays outside this module.  The controller owns
//! the editable `PlayerFile`, previews and the three-way image update intent
//! needed by the application to distinguish an unchanged image from a cleared
//! one.

use crate::classic_gui::{ClassicGuiSkin, IntRect};
use crate::startup_main_menu::StartupTooltip;
use crate::startup_options_dlg::BookFonts;
use crate::startup_portraitsel::{
    PortraitFileEntry, PortraitLocation, PortraitSelAction, PortraitSelCommit,
    PortraitSelController, PortraitSelLabels, PortraitSelResources, PortraitSelSound,
    PortraitThumbnailRequest,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};
use clonk_engine::player_file::PlayerFile;
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{Color, GammaRamp, Surface};
use clonk_gui::Rect as GuiRect;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

/// `C4StartupEditBorderColor` (`src/C4Startup.h:31`).
const STARTUP_EDIT_BORDER_COLOR: u32 = 0x00a4_947a;

/// `C4StartupFontClr` 0xff000000 (`src/C4Startup.h:28`): opaque black.
const STARTUP_FONT_RGBA: [u8; 4] = [0, 0, 0, 255];

/// The half-transparent olive box `C4GUI::Edit::DrawElement` paints behind a
/// selection (`src/C4GuiEdit.cpp:614`).
const SELECTION_BOX_COLOR: u32 = 0x7f7f_7f00;

/// `C4GUI_IconWdt`/`C4GUI_IconHgt` are 40 and `GUIIcons.png` is 240 wide
/// (`src/C4Gui.h:105-106`, `src/C4Gui.cpp:1198-1199`), so the atlas has six
/// columns — the fallback `Icon::GetIconFacet` also assumes
/// (`src/C4GuiLabels.cpp:584`).
const GUI_ICON_COLUMNS: i32 = 6;

/// `C4P_Control_GamePad1` (`src/C4Constants.h:89`): control sets below this
/// index draw `fctKeyboard`, the rest draw `fctGamepad`.
const KEYBOARD_CONTROL_SETS: i32 = 4;

/// `C4GUI::Edit::GetMarginLeft/Right` and `GetMarginTop/Bottom`
/// (`src/C4GuiEdit.h:101-104`).
const EDIT_MARGIN_X: i32 = 4;
const EDIT_MARGIN_Y: i32 = 2;

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
    PortraitLocationChanged {
        index: usize,
        path: PathBuf,
    },
    PortraitSelectorClosed {
        location_index: usize,
    },
    PortraitSelectionRequired,
    ApplyPicture(PortraitSelCommit),
    /// A `GUISound` the nested portrait selector raised, forwarded verbatim so
    /// the host plays it (`C4GuiMenu.cpp:172,418,465,528`).
    GuiSound(PortraitSelSound),
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

/// `C4GUI::ComponentAligner` (`C4Gui.h:1883-1926`, `C4Gui.cpp:1079-1161`).
/// The margins are re-applied on every cut and each cut consumes
/// `size + 2 * margin` from the remaining area along its axis. `take_*`
/// mirrors C++'s `GetFrom*`, renamed because these consume from `self`.
struct Aligner {
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
}

impl Aligner {
    fn new(x: i32, y: i32, w: i32, h: i32, margin_x: i32, margin_y: i32) -> Self {
        Self {
            area: IntRect::new(x, y, w, h),
            margin_x,
            margin_y,
        }
    }

    fn from_rect(area: IntRect, margin_x: i32, margin_y: i32) -> Self {
        Self {
            area,
            margin_x,
            margin_y,
        }
    }

    /// `GetHeight()` returns the raw remaining height — margins are not
    /// subtracted (`C4Gui.h:1914`).
    fn height(&self) -> i32 {
        self.area.h
    }

    fn take_top(&mut self, height: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            self.area.w - self.margin_x * 2,
            height,
        );
        let consumed = height + self.margin_y * 2;
        self.area.y += consumed;
        self.area.h -= consumed;
        out
    }

    fn take_bottom(&mut self, height: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.area.h - height - self.margin_y,
            self.area.w - self.margin_x * 2,
            height,
        );
        self.area.h -= height + self.margin_y * 2;
        out
    }

    fn take_left(&mut self, width: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            width,
            self.area.h - self.margin_y * 2,
        );
        let consumed = width + self.margin_x * 2;
        self.area.x += consumed;
        self.area.w -= consumed;
        out
    }

    fn take_right(&mut self, width: i32) -> IntRect {
        let out = IntRect::new(
            self.area.x + self.area.w - width - self.margin_x,
            self.area.y + self.margin_y,
            width,
            self.area.h - self.margin_y * 2,
        );
        self.area.w -= width + self.margin_x * 2;
        out
    }

    fn all(&self) -> IntRect {
        IntRect::new(
            self.area.x + self.margin_x,
            self.area.y + self.margin_y,
            self.area.w - self.margin_x * 2,
            self.area.h - self.margin_y * 2,
        )
    }

    fn expand_top(&mut self, by: i32) {
        self.area.y -= by;
        self.area.h += by;
    }

    fn expand_left(&mut self, by: i32) {
        self.area.x -= by;
        self.area.w += by;
    }
}

/// Width and height of `C4Startup::Graphics::fctPlrPropBG`
/// (`StartupPlrPropBG.png`), which is also the dialog's size
/// (`C4StartupPlrSelDlg.cpp:1094`).
pub const PAPER_WIDTH: i32 = 365;
pub const PAPER_HEIGHT: i32 = 400;

/// `C4StartupPlrPropertiesDlg::GetMargin*` (`C4StartupPlrSelDlg.h:268-271`).
/// Every child rect below is stated relative to the paper, i.e. with these
/// margins already added to the `ComponentAligner` coordinates.
const MARGIN_LEFT: i32 = 45;
const MARGIN_TOP: i32 = 16;
const MARGIN_RIGHT: i32 = 55;
const MARGIN_BOTTOM: i32 = 30;

/// `C4StartupGraphics::BookFont` / `BookSmallFont` line heights: `C4FT_Main`
/// at `RXFontSize` 14 and `C4FT_MainSmall` at 13 (`C4Startup.cpp:107,117`),
/// rasterized from Endeavour. `book_font_line_heights_match_the_layout`
/// keeps these in step with the real faces.
const BOOK_LINE_HEIGHT: i32 = 22;
const BOOK_SMALL_LINE_HEIGHT: i32 = 20;

/// `C4GUI::ArrowButton::GetDefaultWidth/Height` = `fctBigArrows.Wdt/Hgt`,
/// a 76x40 sheet cut into four phases (`C4Gui.cpp:1209-1210`).
const ARROW_WIDTH: i32 = 19;
const ARROW_HEIGHT: i32 = 40;
/// `C4GUI_ScrollBarHgt` (`C4Gui.h:112`).
const SCROLLBAR_HEIGHT: i32 = 16;
/// `C4GUI_ScrollArrowWdt` (`C4Gui.h:114`) and `C4GUI_ScrollThumbWdt`
/// (`C4Gui.h:116`), which bound a callback scroll bar's pin travel.
const SCROLL_ARROW_WIDTH: i32 = 16;
const SCROLL_THUMB_WIDTH: i32 = 16;
/// `C4GUI_ButtonAreaHgt` (`C4Gui.h:121`): reserved but never drawn into.
const BUTTON_AREA_HEIGHT: i32 = 40;
/// `C4GUI::Edit::GetCustomEditHeight(BookFont)` =
/// `max(22 + 3, C4GUI_MinWoodBarHgt)` (`C4GuiEdit.cpp:114-118`, `C4Gui.h:161`).
const EDIT_HEIGHT: i32 = 25;
/// `BetweenElementDist` (`C4StartupPlrSelDlg.cpp:1115`).
const BETWEEN_ELEMENTS: i32 = 2;
/// `Game.GraphicsResource.fctKeyboard` / `fctGamepad`
/// (`C4GraphicsResource.cpp:201,229`).
const CONTROL_FACET_WIDTH: i32 = 80;
const CONTROL_FACET_HEIGHT: i32 = 36;
/// `Game.GraphicsResource.fctFlagClr`, a `C4FCT_Full` 64x64 facet over
/// `Flag.png` (`C4GraphicsResource.cpp:209,254-255`).
const FLAG_FACET_SIZE: i32 = 64;
/// `C4Startup::Graphics::fctPlrCtrlType` (`C4Startup.cpp:79`): a 128x52 facet
/// over the 256x102 `StartupPlrCtrlType.png`, i.e. a 2x2 phase grid.
const MOVEMENT_FACET_WIDTH: i32 = 128;
const MOVEMENT_FACET_HEIGHT: i32 = 52;

/// Fixed geometry of the 365x400 property paper, derived exactly as
/// `C4StartupPlrPropertiesDlg`'s constructor derives it through
/// `C4GUI::ComponentAligner` (`C4StartupPlrSelDlg.cpp:1116-1235`).
///
/// The dialog does not scale with the window: `C4GUI::Screen::ShowDialog`
/// centers it at `((screen - paper) / 2)` (`C4Gui.cpp:660-676`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerPropertiesLayout {
    pub paper: IntRect,
    pub title: IntRect,
    pub name_label: IntRect,
    pub name: IntRect,
    pub color_label: IntRect,
    pub color_swatch: IntRect,
    pub color_previous: IntRect,
    pub color_next: IntRect,
    pub rgb_sliders: [IntRect; 3],
    pub control_label: IntRect,
    pub picture_label: IntRect,
    pub control_previous: IntRect,
    pub control_next: IntRect,
    pub control_preview: IntRect,
    pub mouse: IntRect,
    pub picture: IntRect,
    pub movement_label: IntRect,
    pub classic_movement: IntRect,
    pub jump_and_run_movement: IntRect,
    pub ok: IntRect,
    pub cancel: IntRect,
}

impl PlayerPropertiesLayout {
    pub fn for_size(width: i32, height: i32) -> Self {
        let paper = IntRect::new(
            (width - PAPER_WIDTH) / 2,
            (height - PAPER_HEIGHT) / 2,
            PAPER_WIDTH,
            PAPER_HEIGHT,
        );
        // `caMain(GetClientRect(), 0, 1, true)`: the client rect zeroed to the
        // origin, so every cut below is offset by the dialog margins.
        let mut main = Aligner::new(
            0,
            0,
            PAPER_WIDTH - MARGIN_LEFT - MARGIN_RIGHT,
            PAPER_HEIGHT - MARGIN_TOP - MARGIN_BOTTOM,
            0,
            1,
        );
        let rect =
            |r: IntRect| r.with_position(paper.x + MARGIN_LEFT + r.x, paper.y + MARGIN_TOP + r.y);

        // `caButtonArea` only reserves space; nothing is placed into it.
        main.take_bottom(BUTTON_AREA_HEIGHT);
        let title = rect(main.take_top(BOOK_LINE_HEIGHT));
        main.expand_top(-BETWEEN_ELEMENTS);
        let name_label = rect(main.take_top(BOOK_SMALL_LINE_HEIGHT));
        let name = rect(main.take_top(EDIT_HEIGHT));
        main.expand_top(-BETWEEN_ELEMENTS);
        let color_label = rect(main.take_top(BOOK_SMALL_LINE_HEIGHT));

        let color_row = main.take_top(ARROW_HEIGHT);
        let mut color = Aligner::from_rect(color_row, 2, 0);
        color.expand_left(2);
        let color_previous = rect(color.take_left(ARROW_WIDTH));
        let flag_width = color.height() * FLAG_FACET_SIZE / FLAG_FACET_SIZE;
        let color_swatch = rect(color.take_left(flag_width));
        let color_next = rect(color.take_left(ARROW_WIDTH));
        let slider_y_diff = (color.height() - 3 * SCROLLBAR_HEIGHT) / 2;
        let red = rect(color.take_top(SCROLLBAR_HEIGHT));
        color.expand_top(-slider_y_diff);
        let green = rect(color.take_top(SCROLLBAR_HEIGHT));
        color.expand_top(-slider_y_diff);
        let blue = rect(color.take_top(SCROLLBAR_HEIGHT));

        main.expand_top(-BETWEEN_ELEMENTS);
        let control_pic_size = ARROW_HEIGHT;
        let control_row =
            main.take_top(control_pic_size + BOOK_SMALL_LINE_HEIGHT + BETWEEN_ELEMENTS);
        let mut control_area = Aligner::from_rect(control_row, 0, 0);
        let mut picture_area = Aligner::from_rect(control_area.take_right(control_pic_size), 0, 0);
        let control_label = rect(control_area.take_top(BOOK_SMALL_LINE_HEIGHT));
        let picture_label_cell = picture_area.take_top(BOOK_SMALL_LINE_HEIGHT);
        control_area.expand_top(-BETWEEN_ELEMENTS);
        picture_area.expand_top(-BETWEEN_ELEMENTS);
        let mut control = Aligner::from_rect(control_area.take_top(control_pic_size), 2, 0);
        let control_previous = rect(control.take_left(ARROW_WIDTH));
        let control_width = control.height() * CONTROL_FACET_WIDTH / CONTROL_FACET_HEIGHT;
        let control_preview = rect(control.take_left(control_width));
        let control_next = rect(control.take_left(ARROW_WIDTH));
        control.expand_left(-10);
        let mouse = rect(control.take_left(control.height()));
        let picture = rect(picture_area.all());

        main.expand_top(-BETWEEN_ELEMENTS);
        let movement_label = rect(main.take_top(BOOK_SMALL_LINE_HEIGHT));
        let mut movement = Aligner::from_rect(main.take_top(MOVEMENT_FACET_HEIGHT), 5, 0);
        let movement_width = movement.height() * MOVEMENT_FACET_WIDTH / MOVEMENT_FACET_HEIGHT;
        let jump_and_run_movement = rect(movement.take_left(movement_width));
        let classic_movement = rect(movement.take_right(movement_width));

        Self {
            paper,
            title,
            name_label,
            name,
            color_label,
            color_swatch,
            color_previous,
            color_next,
            rgb_sliders: [red, green, blue],
            control_label,
            // `ACenter` at the cell's horizontal middle; the drawn rect is
            // narrowed to the text extent around that anchor at paint time.
            picture_label: rect(picture_label_cell),
            control_previous,
            control_next,
            control_preview,
            mouse,
            picture,
            movement_label,
            classic_movement,
            jump_and_run_movement,
            // Both close buttons use paper-absolute rects minus the margins
            // (`C4StartupPlrSelDlg.cpp:1228,1231`), so they land back on the
            // literal paper coordinates. Neither draws anything: the "OK"
            // word and the "x" glyph are painted into `StartupPlrPropBG.png`.
            ok: IntRect::new(paper.x + 147, paper.y + 330, 54, 33),
            cancel: IntRect::new(paper.x + 317, paper.y + 16, 21, 21),
        }
    }

    /// Pin travel of a horizontal callback `C4GUI::ScrollBar`:
    /// `GetMaxScroll()` (`C4Gui.h:900`) with the non-dynamic thumb size.
    pub fn slider_pin_offset(slider: IntRect, value: u8) -> i32 {
        let max_scroll = (slider.w - 2 * SCROLL_ARROW_WIDTH - SCROLL_THUMB_WIDTH).max(0);
        i32::from(value) * max_scroll / 255
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
    IntRect::new(
        button.x + button.w / 2 - w / 2,
        button.y + button.h - 6,
        w,
        h,
    )
}

/// Facet sheets used by [`PlayerPropertiesScreen`], in the same resolution
/// order `C4StartupPlrPropertiesDlg` reaches them.
pub struct PlayerPropertiesAssets {
    /// `StartupPlrPropBG.png` — `C4Startup::Graphics::fctPlrPropBG`. The "OK"
    /// word and the top-right "x" are painted into this sheet; the matching
    /// `CloseIconButton`s carry `Ico_None` and draw nothing themselves.
    pub background: ImageData,
    /// `Config.Graphics.PointFiltering` and `pApp->GetScale()`, carried here
    /// because the nested portrait selector's thumbnail blits obey
    /// `StdGL.cpp:527` like every other textured blit.
    pub point_filtering: bool,
    pub application_scale: f32,
    /// `GUIBigArrows.png` — `fctBigArrows`, four 19x40 phases:
    /// Left, Right, Left-down, Right-down (`C4GuiButton.cpp:262-269`).
    pub big_arrows: ImageData,
    /// `StartupBookScroll.png` — `fctBookScroll`, the 48x48 sheet behind
    /// `sfctBookScrollR/G/B` (`C4Startup.cpp:58-62`).
    pub book_scroll: ImageData,
    /// `GUIIcons.png` — `C4GUI::Resource::fctIcons`, source of `Ico_MouseOff`
    /// (26) and `Ico_MouseOn` (27) (`C4GuiLabels.cpp:577-586`).
    pub icons: ImageData,
    /// `GUIButtonHighlight.png` — the additive focus/hover overlay every
    /// `ArrowButton`/`IconButton` draws (`C4GuiButton.cpp:209-222`).
    pub button_highlight: ImageData,
    /// `Flag.png` — the `ClrByOwner` source of `fctFlagClr`
    /// (`C4GraphicsResource.cpp:209,251-256`).
    pub flag: ImageData,
    /// `Control.png` — `fctKeyboard` is its `(0, 0, 80, 36)` facet
    /// (`C4GraphicsResource.cpp:201`).
    pub control: ImageData,
    /// `Gamepad.png` — `fctGamepad`, 80px phases (`C4GraphicsResource.cpp:229`).
    pub gamepad: Option<ImageData>,
    /// `StartupPlrCtrlType.png` — `fctPlrCtrlType`, a 2x2 grid of 128x52
    /// movement-style phases (`C4Startup.cpp:78-79`).
    pub control_types: Option<ImageData>,
    /// `GUICaption.png`, `GUIButton.png`, `GUIButtonDown.png`: the wooden GUI
    /// skin the nested `C4PortraitSelDlg` draws with. The property paper
    /// itself never uses them.
    pub caption: ImageData,
    pub button: ImageData,
    pub button_down: ImageData,
    /// `GUIContext.png` — the two 16x16 closed/open combo-arrow phases.
    pub context: ImageData,
    /// `GUICheckbox.png` — enabled/disabled unchecked/checked phases.
    pub checkbox: ImageData,
    /// `GUIScroll.png` — permanent portrait-list scrollbar facets.
    pub scroll: ImageData,
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
        self.open_portrait_selector_with_labels(
            locations,
            current_location,
            entries,
            PortraitSelLabels::default(),
        );
    }

    /// As [`Self::open_portrait_selector`], with captions resolved from the
    /// active language table.
    pub fn open_portrait_selector_with_labels(
        &mut self,
        locations: Vec<PortraitLocation>,
        current_location: usize,
        entries: Vec<PortraitFileEntry>,
        labels: PortraitSelLabels,
    ) {
        // The sole C++ caller always passes `true, true`
        // (`C4StartupPlrSelDlg.cpp:1509-1517`). The selector still keeps both
        // presentation-only channels independent after the user changes them.
        let mut selector = PortraitSelController::with_labels(
            locations,
            current_location,
            entries,
            true,
            true,
            labels,
        );
        selector.resize(self.width, self.height);
        if let Some(point) = self.pointer_position {
            selector.handle_pointer_move(point);
        }
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

    pub fn tick_portrait_selector_scrollbar(&mut self) -> bool {
        self.portrait_selector
            .as_mut()
            .is_some_and(PortraitSelController::tick_scrollbar)
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

    /// The dialog's own caption: `IDS_PLR_NEWPLAYER` when creating and
    /// `IDS_DLG_PLAYER2` when editing (`C4StartupPlrSelDlg.cpp:1126-1132`).
    pub const fn title(&self) -> &'static str {
        match self.mode {
            PlayerPropertiesMode::New => "New player",
            PlayerPropertiesMode::Edit { .. } => "Player Properties",
        }
    }

    /// `PrefColorDw` as the `ClrByOwner` modulation colour applied to
    /// `fctFlagClr` and the picture button (`C4StartupPlrSelDlg.cpp:1262-1263`).
    pub fn owner_color(&self) -> Color {
        let color = self.color();
        Color::opaque(
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
        )
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
        self.pointer_position
    }

    /// Returns exactly the tooltips assigned by the native player-properties
    /// dialog. Untipped edits, previews, swatches and OK/Cancel controls do
    /// not inherit descriptions from unrelated siblings.
    pub fn tooltip_at(&self, point: GuiPoint, book_small: &ClonkFont) -> Option<StartupTooltip> {
        if let Some(selector) = self.portrait_selector.as_ref() {
            return selector.tooltip_at(point);
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
        self.pointer_position = position;
        if position.is_none() {
            self.pointer_pressed = None;
        }
        if let Some(selector) = self.portrait_selector.as_mut() {
            match position {
                Some(point) => {
                    selector.handle_pointer_move(point);
                }
                None => {
                    let _ = selector.cancel_interaction();
                }
            }
            return;
        }
        self.hovered = position.and_then(|point| self.hit_control(point));
    }

    pub fn pointer_left(&mut self) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = None;
        self.pointer_pressed = None;
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.pointer_left();
            return self.finish_portrait_selector_actions(actions);
        }
        self.hovered = None;
        Vec::new()
    }

    pub fn cancel_interaction(&mut self) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = None;
        self.pointer_pressed = None;
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.cancel_interaction();
            return self.finish_portrait_selector_actions(actions);
        }
        self.hovered = None;
        Vec::new()
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        self.handle_pointer_move_with_left_down(position, true)
    }

    pub fn handle_pointer_move_with_left_down(
        &mut self,
        position: GuiPoint,
        left_down: bool,
    ) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = Some(position);
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_move_with_left_down(position, left_down);
            return self.finish_portrait_selector_actions(actions);
        }
        self.hovered = self.hit_control(position);
        if self.pointer_pressed.is_some_and(is_slider) {
            self.update_slider_from_pointer(self.pointer_pressed.unwrap(), position);
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = Some(position);
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_down(position);
            return self.finish_portrait_selector_actions(actions);
        }
        self.hovered = self.hit_control(position);
        self.pointer_pressed = self.hovered;
        if let Some(control) = self.hovered {
            // Picture is an IconButton and therefore declines click focus;
            // the modal selector must return to the parent's prior control.
            if control != PlayerPropertiesControl::Picture {
                self.focus = control;
            }
            if is_slider(control) {
                self.update_slider_from_pointer(control, position);
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_right_down(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = Some(position);
        let Some(selector) = self.portrait_selector.as_mut() else {
            return Vec::new();
        };
        let actions = selector.handle_pointer_right_down(position);
        self.finish_portrait_selector_actions(actions)
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = Some(position);
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_pointer_up(position);
            return self.finish_portrait_selector_actions(actions);
        }
        self.hovered = self.hit_control(position);
        let Some(pressed) = self.pointer_pressed.take() else {
            return Vec::new();
        };
        if is_slider(pressed) || self.hovered != Some(pressed) {
            return Vec::new();
        }
        self.activate(pressed)
    }

    pub fn handle_pointer_double_click(
        &mut self,
        position: GuiPoint,
    ) -> Vec<PlayerPropertiesAction> {
        self.pointer_position = Some(position);
        let Some(selector) = self.portrait_selector.as_mut() else {
            return Vec::new();
        };
        let actions = selector.handle_pointer_double_click(position);
        self.finish_portrait_selector_actions(actions)
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
        self.handle_key_down_with_tab_direction(key, false)
    }

    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_key_down_with_tab_direction(key, backwards);
            return self.finish_portrait_selector_actions(actions);
        }
        match key {
            KeyCode::Escape => vec![PlayerPropertiesAction::Cancel],
            KeyCode::Enter => vec![PlayerPropertiesAction::Submit],
            KeyCode::Tab => {
                self.move_focus(backwards);
                Vec::new()
            }
            KeyCode::Down => {
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

    pub fn handle_gamepad_low_down(&mut self) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_gamepad_low_down();
            return self.finish_portrait_selector_actions(actions);
        }
        self.handle_key_down(KeyCode::Enter)
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_gamepad_low_up();
            return self.finish_portrait_selector_actions(actions);
        }
        self.handle_key_up(KeyCode::Enter)
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_gamepad_high_down();
            return self.finish_portrait_selector_actions(actions);
        }
        self.handle_key_down(KeyCode::Escape)
    }

    pub fn handle_gamepad_direction(&mut self, key: KeyCode) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_gamepad_direction(key);
            return self.finish_portrait_selector_actions(actions);
        }
        self.handle_key_down(key)
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<PlayerPropertiesAction> {
        if let Some(selector) = self.portrait_selector.as_mut() {
            let actions = selector.handle_key_up(key);
            return self.finish_portrait_selector_actions(actions);
        }
        Vec::new()
    }

    pub fn handle_portrait_hotkey(
        &mut self,
        character: char,
    ) -> Option<Vec<PlayerPropertiesAction>> {
        let actions = self.portrait_selector.as_mut()?.handle_hotkey(character)?;
        Some(self.finish_portrait_selector_actions(actions))
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
                PortraitSelAction::Cancel => {
                    let location_index = self
                        .portrait_selector
                        .as_ref()
                        .map(PortraitSelController::current_location_index)
                        .unwrap_or_default();
                    self.close_portrait_selector();
                    outer.push(PlayerPropertiesAction::PortraitSelectorClosed { location_index });
                }
                PortraitSelAction::ChangeLocation { index, path } => {
                    outer.push(PlayerPropertiesAction::PortraitLocationChanged { index, path });
                }
                PortraitSelAction::Accept(commit) => {
                    let location_index = self
                        .portrait_selector
                        .as_ref()
                        .map(PortraitSelController::current_location_index)
                        .unwrap_or_default();
                    self.close_portrait_selector();
                    outer.push(PlayerPropertiesAction::PortraitSelectorClosed { location_index });
                    outer.push(PlayerPropertiesAction::ApplyPicture(commit));
                }
                PortraitSelAction::SelectionRequired => {
                    outer.push(PlayerPropertiesAction::PortraitSelectionRequired);
                }
                PortraitSelAction::GuiSound(sound) => {
                    outer.push(PlayerPropertiesAction::GuiSound(sound));
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
    /// Draws the player form and an open portrait selector in immediate
    /// `C4GUI::Window::Draw` order.
    pub fn render(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        fonts: &ClonkFontSet,
        book: &BookFonts,
        controller: &mut PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_player_form(surface, assets, book, controller, gamma);
        Self::render_portrait_selector(surface, assets, fonts, controller, gamma);
    }

    /// Painter order is `Window::Draw`'s: `DrawElement` blits the paper, then
    /// every child in the order `C4StartupPlrPropertiesDlg`'s constructor
    /// added it (`C4StartupPlrSelDlg.cpp:1116-1235`).
    pub fn render_player_form(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        book: &BookFonts,
        controller: &PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        let layout =
            PlayerPropertiesLayout::for_size(surface.width() as i32, surface.height() as i32);
        crate::draw_image_bilinear(surface, &gui_rect(layout.paper), &assets.background, gamma);

        let highlighted =
            |control| controller.focus == control || controller.hovered == Some(control);
        let pressed = |control| controller.pointer_pressed == Some(control);

        // Title and the four section labels: `C4GUI::Label::DrawElement`
        // (`C4GuiLabels.cpp:38-41`) draws at `(x0, rcBounds.y)` with the
        // label's own alignment, in `C4StartupFontClr` opaque black.
        book.book.draw_with_gamma(
            surface,
            layout.title.x,
            layout.title.y,
            controller.title(),
            STARTUP_FONT_RGBA,
            TextAlign::Left,
            false,
            gamma,
        );
        for (rect, text) in [
            (layout.name_label, "Name:"),
            (layout.color_label, "Color:"),
            (layout.control_label, "Control:"),
            (layout.movement_label, "Movement:"),
        ] {
            book.book_small.draw_with_gamma(
                surface,
                rect.x,
                rect.y,
                text,
                STARTUP_FONT_RGBA,
                TextAlign::Left,
                false,
                gamma,
            );
        }
        book.book_small.draw_with_gamma(
            surface,
            layout.picture_label.x + layout.picture_label.w / 2,
            layout.picture_label.y,
            "Picture:",
            STARTUP_FONT_RGBA,
            TextAlign::Center,
            false,
            gamma,
        );

        draw_name_edit(
            surface,
            book,
            layout.name,
            controller.name(),
            // `Edit::OnGetFocus` selects the whole text and `OnLooseFocus`
            // clears it (`C4GuiEdit.cpp:543-556`); hovering changes nothing.
            controller.focus == PlayerPropertiesControl::Name,
            gamma,
        );

        // Colour row: left arrow, `fctFlagClr` tinted by the player colour,
        // right arrow, then the three `sfctBookScrollR/G/B` sliders.
        draw_arrow_button(
            surface,
            assets,
            layout.color_previous,
            ArrowPhase::Left,
            pressed(PlayerPropertiesControl::ColorPrevious),
            highlighted(PlayerPropertiesControl::ColorPrevious),
            gamma,
        );
        let (flag_base, flag_overlay) =
            retained_owner_colored_flag(&assets.flag, controller.owner_color());
        let flag_rect = aspect_fitted((FLAG_FACET_SIZE, FLAG_FACET_SIZE), layout.color_swatch);
        crate::draw_image_bilinear(surface, &gui_rect(flag_rect), &flag_base, gamma);
        crate::draw_image_bilinear(surface, &gui_rect(flag_rect), &flag_overlay, gamma);
        draw_arrow_button(
            surface,
            assets,
            layout.color_next,
            ArrowPhase::Right,
            pressed(PlayerPropertiesControl::ColorNext),
            highlighted(PlayerPropertiesControl::ColorNext),
            gamma,
        );
        for (index, (component, pin_row)) in [
            (PlayerColorComponent::Red, 0),
            (PlayerColorComponent::Green, 1),
            (PlayerColorComponent::Blue, 2),
        ]
        .into_iter()
        .enumerate()
        {
            let slider = layout.rgb_sliders[index];
            let value = controller.color_component(component);
            crate::startup_options_dlg::draw_horizontal_book_scrollbar(
                surface,
                &assets.book_scroll,
                &slider,
                PlayerPropertiesLayout::slider_pin_offset(slider, value),
                false,
                false,
                // `ScrollBarFacets::Set(fctBookScroll, 1..3)`
                // (`C4Gui.cpp:210-211`) takes the pin from column 32.
                (32, 16 * pin_row),
                gamma,
            );
        }

        // Control row: left arrow, the selected control-set image, right
        // arrow, the mouse toggle, and the picture button on the right.
        draw_arrow_button(
            surface,
            assets,
            layout.control_previous,
            ArrowPhase::Left,
            pressed(PlayerPropertiesControl::ControlPrevious),
            highlighted(PlayerPropertiesControl::ControlPrevious),
            gamma,
        );
        draw_control_set_facet(surface, assets, layout.control_preview, controller, gamma);
        draw_arrow_button(
            surface,
            assets,
            layout.control_next,
            ArrowPhase::Right,
            pressed(PlayerPropertiesControl::ControlNext),
            highlighted(PlayerPropertiesControl::ControlNext),
            gamma,
        );
        draw_icon_button(
            surface,
            assets,
            layout.mouse,
            // `Ico_MouseOn` 27 / `Ico_MouseOff` 26 (`C4Gui.h:716-717`).
            Some(if controller.player.pref_mouse { 27 } else { 26 }),
            pressed(PlayerPropertiesControl::Mouse),
            highlighted(PlayerPropertiesControl::Mouse),
            gamma,
        );
        draw_picture_button(
            surface,
            assets,
            layout.picture,
            controller,
            pressed(PlayerPropertiesControl::Picture),
            highlighted(PlayerPropertiesControl::Picture),
            gamma,
        );

        // Movement row: each label is added before its button, so the button's
        // facet paints over the label's descenders exactly as in C++.
        let [classic_label, jump_label] = layout.movement_label_rects(&book.book_small);
        for (label, text, button, control, phase) in [
            (
                jump_label,
                "Jump'n'Run",
                layout.jump_and_run_movement,
                PlayerPropertiesControl::JumpAndRunMovement,
                // `GetPhase(PrefControlStyle ? 1 : 0, 1)`
                // (`C4StartupPlrSelDlg.cpp:1343`).
                (i32::from(controller.player.pref_control_style), 1),
            ),
            (
                classic_label,
                "Classic",
                layout.classic_movement,
                PlayerPropertiesControl::ClassicMovement,
                // `GetPhase(PrefControlStyle ? 0 : 1, 0)`
                // (`C4StartupPlrSelDlg.cpp:1344`).
                (i32::from(!controller.player.pref_control_style), 0),
            ),
        ] {
            book.book_small.draw_with_gamma(
                surface,
                label.x + label.w / 2,
                label.y,
                text,
                STARTUP_FONT_RGBA,
                TextAlign::Center,
                false,
                gamma,
            );
            draw_movement_button(
                surface,
                assets,
                button,
                phase,
                pressed(control),
                highlighted(control),
                gamma,
            );
        }

        // OK and Cancel are `CloseIconButton`s carrying `Ico_None`: the words
        // are part of `StartupPlrPropBG.png`, so only the additive highlight
        // is ever drawn (`C4GuiButton.cpp:205-222`).
        for (rect, control) in [
            (layout.ok, PlayerPropertiesControl::Ok),
            (layout.cancel, PlayerPropertiesControl::Cancel),
        ] {
            draw_icon_button(
                surface,
                assets,
                rect,
                None,
                pressed(control),
                highlighted(control),
                gamma,
            );
        }
    }

    /// Draws the modal selector after every player-form element, matching the
    /// dialog insertion and traversal order of `C4GUI::Screen::ShowDialog`
    /// (`C4Gui.cpp:573-585`, `C4GuiContainers.cpp:33-44`).
    pub fn render_portrait_selector(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        fonts: &ClonkFontSet,
        controller: &mut PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        if let Some(selector) = controller.portrait_selector.as_mut() {
            selector.render(
                surface,
                Self::portrait_selector_resources(assets, fonts),
                gamma,
            );
        }
    }

    pub fn render_portrait_selector_dialog(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        fonts: &ClonkFontSet,
        controller: &mut PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        if let Some(selector) = controller.portrait_selector.as_mut() {
            selector.render_dialog(
                surface,
                Self::portrait_selector_resources(assets, fonts),
                gamma,
            );
        }
    }

    pub fn render_portrait_location_popup(
        surface: &mut Surface,
        assets: &PlayerPropertiesAssets,
        fonts: &ClonkFontSet,
        controller: &PlayerPropertiesController,
        gamma: Option<&GammaRamp>,
    ) {
        if let Some(selector) = controller.portrait_selector.as_ref() {
            selector.render_location_popup(
                surface,
                Self::portrait_selector_resources(assets, fonts),
                gamma,
            );
        }
    }

    fn portrait_selector_resources<'a>(
        assets: &'a PlayerPropertiesAssets,
        fonts: &'a ClonkFontSet,
    ) -> PortraitSelResources<'a> {
        PortraitSelResources {
            skin: ClassicGuiSkin::new(
                &assets.caption,
                &assets.button,
                &assets.button_down,
                Some(&assets.button_highlight),
            ),
            fonts,
            icons: &assets.icons,
            context: &assets.context,
            checkbox: &assets.checkbox,
            scroll: &assets.scroll,
            control: &assets.control,
            button_highlight: &assets.button_highlight,
            point_filtering: assets.point_filtering,
            application_scale: assets.application_scale,
        }
    }
}

/// `fctBigArrows` phase indices: `ArrowFct` plus `Down` when pressed
/// (`C4Gui.h:1137`, `C4GuiButton.cpp:266-268`).
#[derive(Clone, Copy)]
enum ArrowPhase {
    Left = 0,
    Right = 1,
}

/// `ArrowButton::DrawElement` (`C4GuiButton.cpp:255-269`).
fn draw_arrow_button(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    direction: ArrowPhase,
    down: bool,
    highlight: bool,
    gamma: Option<&GammaRamp>,
) {
    if highlight {
        draw_button_highlight(surface, assets, rect, gamma);
    }
    let phase = direction as i32 + if down { 2 } else { 0 };
    draw_facet_phase(
        surface,
        &assets.big_arrows,
        (phase * ARROW_WIDTH, 0),
        (ARROW_WIDTH, ARROW_HEIGHT),
        rect,
        gamma,
    );
}

/// `IconButton::DrawElement` (`C4GuiButton.cpp:205-231`) for a `GUIIcons`
/// cell, or for `Ico_None` (highlight only).
fn draw_icon_button(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    icon: Option<i32>,
    down: bool,
    highlight: bool,
    gamma: Option<&GammaRamp>,
) {
    if highlight {
        draw_button_highlight(surface, assets, rect, gamma);
    }
    if let Some(icon) = icon {
        // `Icon::GetIconFacet`: square cells, `iXMax` columns per row
        // (`C4GuiLabels.cpp:577-586`).
        let cell = assets.icons.width() as i32 / GUI_ICON_COLUMNS;
        let (column, row) = (icon % GUI_ICON_COLUMNS, icon / GUI_ICON_COLUMNS);
        draw_facet_phase(
            surface,
            &assets.icons,
            (column * cell, row * cell),
            (cell, cell),
            rect,
            gamma,
        );
    }
    if down {
        draw_button_highlight(surface, assets, rect, gamma);
    }
}

/// The picture button's facet: the live big icon when one exists, else
/// `Ico_Player` (`C4StartupPlrSelDlg.cpp:1203,1520-1530`). `SetColor` tints it
/// with the player colour (`C4StartupPlrSelDlg.cpp:1263`).
fn draw_picture_button(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    controller: &PlayerPropertiesController,
    down: bool,
    highlight: bool,
    gamma: Option<&GammaRamp>,
) {
    if highlight {
        draw_button_highlight(surface, assets, rect, gamma);
    }
    match controller.big_icon_preview() {
        Some(icon) => crate::draw_image_bilinear(surface, &gui_rect(rect), icon, gamma),
        // `Ico_Player` is icon 9 (`C4Gui.h:697`).
        None => draw_icon_button(surface, assets, rect, Some(9), false, false, gamma),
    }
    if down {
        draw_button_highlight(surface, assets, rect, gamma);
    }
}

/// The movement `IconButton`s take a `fctPlrCtrlType` phase rather than a GUI
/// icon (`C4StartupPlrSelDlg.cpp:1340-1345`).
fn draw_movement_button(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    phase: (i32, i32),
    down: bool,
    highlight: bool,
    gamma: Option<&GammaRamp>,
) {
    if highlight {
        draw_button_highlight(surface, assets, rect, gamma);
    }
    if let Some(sheet) = assets.control_types.as_ref() {
        draw_facet_phase(
            surface,
            sheet,
            (
                phase.0 * MOVEMENT_FACET_WIDTH,
                phase.1 * MOVEMENT_FACET_HEIGHT,
            ),
            (MOVEMENT_FACET_WIDTH, MOVEMENT_FACET_HEIGHT),
            rect,
            gamma,
        );
    }
    if down {
        draw_button_highlight(surface, assets, rect, gamma);
    }
}

/// `pCtrlImg`: `fctKeyboard` for keyboard sets and `fctGamepad` beyond them,
/// advanced by one facet width per set (`C4StartupPlrSelDlg.cpp:1309-1315`).
fn draw_control_set_facet(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    controller: &PlayerPropertiesController,
    gamma: Option<&GammaRamp>,
) {
    let set = controller.player.pref_control;
    let (sheet, index) = if set < KEYBOARD_CONTROL_SETS {
        (&assets.control, set)
    } else {
        match assets.gamepad.as_ref() {
            Some(gamepad) => (gamepad, set - KEYBOARD_CONTROL_SETS),
            None => (&assets.control, 0),
        }
    };
    draw_facet_phase(
        surface,
        sheet,
        (index * CONTROL_FACET_WIDTH, 0),
        (CONTROL_FACET_WIDTH, CONTROL_FACET_HEIGHT),
        aspect_fitted((CONTROL_FACET_WIDTH, CONTROL_FACET_HEIGHT), rect),
        gamma,
    );
}

/// `GetRes()->fctButtonHighlight.DrawX` under `C4GFXBLIT_ADDITIVE`
/// (`C4GuiButton.cpp:210-213`).
fn draw_button_highlight(
    surface: &mut Surface,
    assets: &PlayerPropertiesAssets,
    rect: IntRect,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(
        surface,
        &gui_rect(rect),
        &crate::startup_options_dlg::retained_blackened_image(&assets.button_highlight),
        gamma,
    );
}

thread_local! {
    /// `fctFlagClr` is built once by `C4Surface::CreateColorByOwner` and only
    /// re-tinted by `SetDrawColor`, so the port keeps one stable `ImageData`
    /// pair per (sheet, colour) instead of re-uploading textures every frame.
    static OWNER_COLORED_FLAGS: RefCell<
        HashMap<(clonk_graphics::GpuTextureId, u32), (ImageData, ImageData)>,
    > = RefCell::new(HashMap::new());
}

/// `C4Surface::CreateColorByOwner` (`C4Surface.cpp:297-327`) splits a sheet in
/// two, and `CStdDDraw::Blit` then draws two independently filtered quads —
/// the base unmodulated, the overlay modulated by `ClrByOwnerClr`
/// (`StdDDraw2.cpp:787-806`). Filtering therefore happens *before* the owner
/// tint combines them, which is what keeps the flag's antialiased border
/// bit-identical; tinting a single merged sheet does not reproduce it.
///
/// Returns `(base, tinted_overlay)`:
/// * base — every non-owner pixel, owner positions forced to transparent black
///   by `SetPixDw` (`C4Surface.cpp:775`);
/// * overlay — the memset-`0xff` transparent white a fresh surface starts with
///   (`C4Surface.cpp:1155`), with owner pixels replaced by the gray
///   `ClrByOwner` leaves behind (`C4Surface.cpp:291-293`), every channel then
///   modulated by the owner colour.
fn retained_owner_colored_flag(flag: &ImageData, owner: Color) -> (ImageData, ImageData) {
    let key = (
        flag.gpu_texture_id(),
        u32::from_be_bytes([owner.a, owner.r, owner.g, owner.b]),
    );
    OWNER_COLORED_FLAGS.with(|cache| {
        if let Some(cached) = cache.borrow().get(&key).cloned() {
            return cached;
        }
        // `ReadPNG` blackens fully transparent texels (`C4Surface.cpp:972`)
        // before the split ever runs.
        let source = crate::startup_options_dlg::retained_blackened_image(flag);
        let modulate =
            |channel: u8, value: u8| ((u16::from(channel) * u16::from(value)) / 255) as u8;
        let mut base = Vec::with_capacity(source.pixels().len());
        let mut overlay = Vec::with_capacity(source.pixels().len());
        for pixel in source.pixels().chunks_exact(4) {
            let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
            match crate::hud::clr_by_owner_gray(i32::from(r), i32::from(g), i32::from(b)) {
                Some(gray) => {
                    base.extend_from_slice(&[0, 0, 0, 0]);
                    overlay.extend_from_slice(&[
                        modulate(owner.r, gray),
                        modulate(owner.g, gray),
                        modulate(owner.b, gray),
                        a,
                    ]);
                }
                None => {
                    base.extend_from_slice(&[r, g, b, a]);
                    overlay.extend_from_slice(&[owner.r, owner.g, owner.b, 0]);
                }
            }
        }
        let split = (
            ImageData::new(source.width(), source.height(), base),
            ImageData::new(source.width(), source.height(), overlay),
        );
        cache.borrow_mut().insert(key, split.clone());
        split
    })
}

/// `C4GUI::Picture::DrawElement` with `fAspect` (`C4GuiLabels.cpp:353-382`):
/// `C4Facet::Draw` fits the source's aspect ratio inside `dest` and centers
/// the shortfall on the scaled axis (`C4Facet.cpp:113-141`).
fn aspect_fitted(source_size: (i32, i32), dest: IntRect) -> IntRect {
    let (source_w, source_h) = source_size;
    if source_w <= 0 || source_h <= 0 {
        return dest;
    }
    if source_w * dest.h > dest.w * source_h {
        let h = source_h * dest.w / source_w;
        dest.with_vertical(dest.y + (dest.h - h) / 2, h)
    } else {
        let w = source_w * dest.h / source_h;
        dest.with_horizontal(dest.x + (dest.w - w) / 2, w)
    }
}

/// `C4Facet::DrawX` of one sheet cell into `dest`.
fn draw_facet_phase(
    surface: &mut Surface,
    sheet: &ImageData,
    source_origin: (i32, i32),
    source_size: (i32, i32),
    dest: IntRect,
    gamma: Option<&GammaRamp>,
) {
    if dest.w <= 0 || dest.h <= 0 || source_size.0 <= 0 || source_size.1 <= 0 {
        return;
    }
    crate::classic_gui::draw_facet_stretch(
        surface,
        sheet,
        (
            source_origin.0 as f32,
            source_origin.1 as f32,
            source_size.0 as f32,
            source_size.1 as f32,
        ),
        (dest.x as f32, dest.y as f32, dest.w as f32, dest.h as f32),
        gamma,
    );
}

/// `C4GUI::Edit::DrawElement` (`C4GuiEdit.cpp:561-627`) with the startup
/// colours: `C4StartupEditBGColor` is fully transparent so the paper shows
/// through, the border is drawn as two nested `DrawFrameDw` passes, a focused
/// edit shows its select-all highlight, and the text is vertically centered.
fn draw_name_edit(
    surface: &mut Surface,
    book: &BookFonts,
    rect: IntRect,
    text: &str,
    focused: bool,
    gamma: Option<&GammaRamp>,
) {
    draw_name_edit_frames(surface, rect, gamma);
    // `Edit::GetMargin*` (`C4GuiEdit.h:101-104`) insets the client rect the
    // text and selection are laid out in.
    let client = IntRect::new(
        rect.x + EDIT_MARGIN_X,
        rect.y + EDIT_MARGIN_Y,
        rect.w - EDIT_MARGIN_X * 2,
        rect.h - EDIT_MARGIN_Y * 2,
    );
    let line_height = book.book.line_height;
    let (text_y, selection_height) = if client.h <= line_height {
        // "very narrow edit field: use all of it" (`C4GuiEdit.cpp:580-585`).
        (client.y, client.h)
    } else {
        (client.y + (client.h - line_height) / 2 + 1, line_height - 2)
    };
    if focused && !text.is_empty() {
        let width = book.book.measure(text, false).0;
        crate::startup_options_dlg::fill_box_dw(
            surface,
            client.x,
            text_y,
            client.x + width - 1,
            text_y + selection_height - 1,
            SELECTION_BOX_COLOR,
            gamma,
        );
    }
    book.book.draw_with_gamma(
        surface,
        client.x,
        text_y - 1,
        text,
        STARTUP_FONT_RGBA,
        TextAlign::Left,
        false,
        gamma,
    );
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_name_edit_keeps_both_native_draw_frame_passes() {
        let mut surface = Surface::new(32, 16, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_name_edit_frames(&mut surface, IntRect::new(2, 2, 20, 10), None);

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

    /// The layout constants are stated as literals because `for_size` has no
    /// font handy; this keeps them honest against the real faces
    /// (`C4Startup.cpp:107,117`).
    #[test]
    fn book_font_line_heights_match_the_layout() {
        let ttf =
            std::fs::read(crate::test_support::repo_root().join("planet/System.c4g/Endeavour.ttf"))
                .expect("read Endeavour.ttf");
        let book = crate::startup_options_dlg::build_book_fonts(&ttf).expect("build book fonts");
        assert_eq!(book.book.line_height, BOOK_LINE_HEIGHT);
        assert_eq!(book.book_small.line_height, BOOK_SMALL_LINE_HEIGHT);
    }

    /// Every rect of `C4StartupPlrPropertiesDlg`'s constructor
    /// (`C4StartupPlrSelDlg.cpp:1116-1235`), relative to the 365x400
    /// `fctPlrPropBG` top-left. Verified against a 1280x720 C++ capture.
    #[test]
    fn layout_matches_the_cpp_component_aligner() {
        let layout = PlayerPropertiesLayout::for_size(1280, 720);
        // `C4GUI::Screen::ShowDialog` centers an exclusive dialog
        // (`C4Gui.cpp:660-676`); the paper never scales with the window.
        assert_eq!(layout.paper, IntRect::new(457, 160, 365, 400));
        let paper = |x: i32, y: i32, w: i32, h: i32| IntRect::new(457 + x, 160 + y, w, h);
        for (name, got, want) in [
            ("title", layout.title, paper(45, 17, 265, 22)),
            ("name label", layout.name_label, paper(45, 43, 265, 20)),
            ("name edit", layout.name, paper(45, 65, 265, 25)),
            ("color label", layout.color_label, paper(45, 94, 265, 20)),
            ("color prev", layout.color_previous, paper(45, 116, 19, 40)),
            ("color swatch", layout.color_swatch, paper(68, 116, 40, 40)),
            ("color next", layout.color_next, paper(112, 116, 19, 40)),
            ("slider R", layout.rgb_sliders[0], paper(135, 116, 173, 16)),
            ("slider G", layout.rgb_sliders[1], paper(135, 128, 173, 16)),
            ("slider B", layout.rgb_sliders[2], paper(135, 140, 173, 16)),
            (
                "control label",
                layout.control_label,
                paper(45, 160, 225, 20),
            ),
            (
                "picture label",
                layout.picture_label,
                paper(270, 160, 40, 20),
            ),
            (
                "control prev",
                layout.control_previous,
                paper(47, 182, 19, 40),
            ),
            (
                "control image",
                layout.control_preview,
                paper(70, 182, 88, 40),
            ),
            ("control next", layout.control_next, paper(162, 182, 19, 40)),
            ("mouse", layout.mouse, paper(195, 182, 40, 40)),
            ("picture", layout.picture, paper(270, 182, 40, 40)),
            (
                "movement label",
                layout.movement_label,
                paper(45, 226, 265, 20),
            ),
            (
                "jump'n'run",
                layout.jump_and_run_movement,
                paper(50, 248, 128, 52),
            ),
            ("classic", layout.classic_movement, paper(177, 248, 128, 52)),
            ("ok", layout.ok, paper(147, 330, 54, 33)),
            ("cancel", layout.cancel, paper(317, 16, 21, 21)),
        ] {
            assert_eq!(got, want, "{name}");
        }
    }

    /// `ScrollBar::SetScrollPos` over `GetMaxScroll()`
    /// (`C4Gui.h:900,923`) with the default `iCBMaxRange` of 256.
    #[test]
    fn slider_pins_travel_over_the_native_scroll_range() {
        let slider = PlayerPropertiesLayout::for_size(1280, 720).rgb_sliders[0];
        assert_eq!(PlayerPropertiesLayout::slider_pin_offset(slider, 0), 0);
        // 173 - 2*16 - 16 = 125 pixels of travel.
        assert_eq!(PlayerPropertiesLayout::slider_pin_offset(slider, 255), 125);
        assert_eq!(PlayerPropertiesLayout::slider_pin_offset(slider, 200), 98);
    }

    /// `C4Facet::Draw` with `fAspect` (`C4Facet.cpp:120-134`).
    #[test]
    fn aspect_fit_matches_the_native_facet_letterboxing() {
        let dest = IntRect::new(70, 182, 88, 40);
        // fctKeyboard 80x36 into 88x40 scales height: 36*88/80 = 39.
        assert_eq!(
            aspect_fitted((80, 36), dest),
            dest.with_height(39),
            "keyboard"
        );
        // fctFlagClr 64x64 into 40x40 needs no letterboxing.
        let square = IntRect::new(68, 116, 40, 40);
        assert_eq!(aspect_fitted((64, 64), square), square, "flag");
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
    fn picture_pointer_activation_preserves_parent_name_focus() {
        // Picture is an IconButton, and buttons decline click focus. Closing
        // the modal child therefore restores the parent's prior Name focus
        // (`C4StartupPlrSelDlg.cpp:1202-1205`, `C4Gui.h:1058-1075`,
        // `C4GuiContainers.cpp:695-712`, `C4Gui.cpp:608-625`).
        let mut state = controller();
        let picture = state.layout().picture;
        let point = GuiPoint::new(
            (picture.x + picture.w / 2) as f32,
            (picture.y + picture.h / 2) as f32,
        );

        assert!(state.handle_pointer_down(point).is_empty());
        assert_eq!(state.focused_control(), PlayerPropertiesControl::Name);
        assert_eq!(
            state.handle_pointer_up(point),
            vec![PlayerPropertiesAction::ChoosePicture]
        );
        state.open_portrait_selector(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
        );
        assert_eq!(
            state.handle_key_down(KeyCode::Escape),
            vec![PlayerPropertiesAction::PortraitSelectorClosed { location_index: 0 }]
        );
        assert!(state.portrait_selector().is_none());
        assert_eq!(state.focused_control(), PlayerPropertiesControl::Name);
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
    fn portrait_selector_transitions_preserve_the_global_mouse_coordinate() {
        // Modal ShowDialog/Close keep Screen's single CMouse coordinate; a
        // stationary button event after either transition still routes at the
        // last position (`C4Gui.cpp:608-689`, `C4MouseControl.cpp:145-188`).
        let mut state = controller();
        state.resize(640, 480);
        let picture = state.layout().picture;
        let opening_point = GuiPoint::new(
            (picture.x + picture.w / 2) as f32,
            (picture.y + picture.h / 2) as f32,
        );
        state.handle_pointer_move(opening_point);
        state.open_portrait_selector(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
        );
        assert_eq!(state.pointer_position(), Some(opening_point));

        let cancel = crate::startup_portraitsel::portrait_sel_layout(640, 480, 1).cancel;
        let closing_point = GuiPoint::new(
            (cancel.x + cancel.w / 2) as f32,
            (cancel.y + cancel.h / 2) as f32,
        );
        state.handle_pointer_move(closing_point);
        state.handle_pointer_down(closing_point);
        assert_eq!(
            state.handle_pointer_up(closing_point),
            vec![
                PlayerPropertiesAction::GuiSound(PortraitSelSound::Click),
                PlayerPropertiesAction::PortraitSelectorClosed { location_index: 0 },
            ]
        );
        assert!(state.portrait_selector().is_none());
        assert_eq!(state.pointer_position(), Some(closing_point));
    }

    #[test]
    fn open_portrait_selector_defaults_both_image_channels_on() {
        // The only C++ caller passes true for fSetPicture and fSetBigIcon,
        // independently of whether either preview currently exists
        // (`C4StartupPlrSelDlg.cpp:1509-1517`).
        let mut state = controller();
        state.open_portrait_selector(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
        );

        let selector = state.portrait_selector().expect("portrait selector");
        assert!(selector.set_picture());
        assert!(selector.set_big_icon());

        let mut state = PlayerPropertiesController::edit(
            0,
            PlayerFile::default(),
            "",
            Some(ImageData::new(1, 1, vec![1, 2, 3, 255])),
            None,
        );
        state.open_portrait_selector(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
        );
        let selector = state.portrait_selector().expect("portrait selector");
        assert!(selector.set_picture());
        assert!(selector.set_big_icon());
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
        for rect in [layout.name, layout.color_swatch, layout.ok, layout.cancel] {
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

        // The native OK and Cancel rects are hard-coded well clear of the
        // movement row (`C4StartupPlrSelDlg.cpp:1228,1231`), so neither can
        // occlude a movement label.
        for (label, other) in [(classic_label, layout.ok), (jump_label, layout.cancel)] {
            let overlaps = label.x < other.x + other.w
                && other.x < label.x + label.w
                && label.y < other.y + other.h
                && other.y < label.y + label.h;
            assert!(!overlaps, "movement labels never reach OK/Cancel");
        }

        state.open_portrait_selector(
            vec![PortraitLocation::new("User", "/portraits")],
            0,
            Vec::new(),
        );
        let selector_layout = crate::startup_portraitsel::portrait_sel_layout(800, 600, 1);
        assert_eq!(
            state.tooltip_at(center(selector_layout.close), &book_small),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            "the nested selector replaces rather than suppresses tooltip targets"
        );
        assert_eq!(
            state.tooltip_at(center(layout.picture), &book_small),
            None,
            "the active modal selector cannot leak a parent-form tooltip"
        );
    }
}
