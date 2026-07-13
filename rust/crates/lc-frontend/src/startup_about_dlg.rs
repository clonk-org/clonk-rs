//! Pixel-parity renderer for one C++ startup dialog (see
//! `rust/target/parity-specs/`). Implemented against the engine's F9
//! reference captures; owned by its implementation agent.
//!
//! Dialog: `C4StartupAboutDlg` (C4StartupAboutDlg.cpp). All geometry mirrors
//! the C++ integer math exactly; see `rust/target/parity-specs/about.md`.

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::classic_gui::{draw_3d_frame, draw_clipped_text, draw_engine_box};
use crate::message_dialog::break_message;
use crate::startup_main_menu::{draw_bar, IntRect};
use crate::{GuiPoint, ImageData, KeyCode};
use lc_graphics::clonk_font::{ClonkFont, TextAlign};
use lc_graphics::{GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

/// Endeavour TitleFont (22px) line height (StdFont.cpp:351 metrics).
const TITLE_LINE_HEIGHT: i32 = 34;
/// Endeavour MiniFont (12px) line height.
const MINI_LINE_HEIGHT: i32 = 18;
/// C4GUI_ButtonHgt (C4Gui.h:108).
const BUTTON_HGT: i32 = 32;
/// C4GUI_ScrollBarWdt (C4Gui.h:111).
const SCROLL_BAR_WDT: i32 = 16;
/// Endeavour TextFont line height used by `LicenseTab` labels.
const TEXT_LINE_HEIGHT: i32 = 22;
/// C4GUI_DefaultListSpacing between consecutive ListBox entries.
const LIST_ROW_SPACING: i32 = 1;
const LICENSE_TAB_PITCH: i32 = TEXT_LINE_HEIGHT + LIST_ROW_SPACING;

/// Bounds of the second About page's `LicenseWindow` children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LicensePageLayout {
    /// The page-sized `LicenseWindow` itself.
    pub window: IntRect,
    /// Left-hand license-title `ListBox`.
    pub tabs: IntRect,
    /// Right-hand license-body `TextWindow`.
    pub text: IntRect,
}

/// One credits section: caption label plus its TextWindow box, both in
/// screen pixels (client-relative rects offset by the client origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionLayout {
    /// Top-left of the CaptionFont label (DrawCaption,
    /// C4StartupAboutDlg.cpp:343-349; ALeft at the section rect origin).
    pub caption_pos: (i32, i32),
    /// TextWindow bounds: the section rect with the top inset by the
    /// TitleFont line height 34 (CreateTextWindowWithText,
    /// C4StartupAboutDlg.cpp:325-329).
    pub textbox: IntRect,
}

/// Pixel-exact C4StartupAboutDlg geometry, all in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AboutLayout {
    /// Fullscreen-dialog client rect (C4GuiDialogs.cpp:816-823,858-862).
    pub client: IntRect,
    /// ACenter anchor of the "&About" title label: x = client middle, y
    /// centers the TitleFont in the 50px title strip
    /// (C4GuiDialogs.cpp:834-850).
    pub title_anchor: (i32, i32),
    /// ARight anchor of the trademark MiniFont label
    /// (C4StartupAboutDlg.cpp:274-275).
    pub trademark_anchor: (i32, i32),
    /// Back, Update, Licenses buttons (C4StartupAboutDlg.cpp:277-283).
    pub buttons: [IntRect; 3],
    /// Credits sections in draw order: Game Design, Engine and Tools,
    /// Scripting, Additional Art, Music, Voice, Web
    /// (C4StartupAboutDlg.cpp:288-301).
    pub sections: [SectionLayout; 7],
    /// Second-page license list and text geometry.
    pub licenses: LicensePageLayout,
}

/// Computes the C4StartupAboutDlg layout for a `w`x`h` screen, mirroring
/// the ComponentAligner math of C4StartupAboutDlg.cpp:262-301 with
/// C4GuiDialogs.cpp:816-823,858-862 (fullscreen margins) and
/// C4Gui.cpp:1026-1090 (GetFromBottom/GetGridCell).
pub fn about_layout(w: i32, h: i32) -> AboutLayout {
    // Fullscreen dialog margins (C4GuiDialogs.cpp:858-862); the top adds the
    // 50px title strip (C4GUI_FullscreenDlg_TitleHeight, C4Gui.h:163).
    let margin_x = if w < 500 { 2 } else { w / 50 };
    let margin_y = if h < 320 { 2 } else { h * 2 / 75 };
    let margin_top = 50 + margin_y;
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: w - 2 * margin_x,
        h: h - margin_top - margin_y,
    };
    let (cw, ch) = (client.w, client.h);

    // Title label: ACenter at clientWdt/2, vertically centered in the title
    // strip (C4GuiDialogs.cpp:834-850) — in screen coords y = 50/2 - lh/2.
    let title_anchor = (client.x + cw / 2, 50 / 2 - TITLE_LINE_HEIGHT / 2);

    // caButtons = caMain.GetFromBottom(ch/8) (C4StartupAboutDlg.cpp:270-271).
    let buttons_area_y = ch - ch / 8;
    let mut buttons_area_h = ch / 8;

    // Trademark label strip: caButtons.GetFromBottom(MiniFont lh)
    // (C4StartupAboutDlg.cpp:274-275); ARight anchors at the rect's right.
    let trademark_anchor = (
        client.x + cw,
        client.y + buttons_area_y + buttons_area_h - MINI_LINE_HEIGHT,
    );
    buttons_area_h -= MINI_LINE_HEIGHT;

    // GetGridCell(x, xn, 0, 1, cw/4, 32, fCenterPos=true) (C4Gui.cpp:1059-1090):
    // cell = areaW/xn, the 307x32 button centered inside its cell.
    let button_w = cw / 4;
    // One row (iSectYMax=1): the cell is the full remaining strip height.
    let button_y = client.y + buttons_area_y + (buttons_area_h - BUTTON_HGT) / 2;
    let grid_cell = |sect: i32, sects: i32| -> IntRect {
        let cell_w = cw / sects;
        IntRect {
            x: client.x + sect * cell_w + (cell_w - button_w) / 2,
            y: button_y,
            w: button_w,
            h: BUTTON_HGT,
        }
    };
    let buttons = [grid_cell(0, 3), grid_cell(2, 4), grid_cell(3, 4)];

    // Credits columns: three GetFromLeft(cw/3) aligners over the remaining
    // (0,0,cw,ch-ch/8) area (C4StartupAboutDlg.cpp:288-301).
    let col_w = cw / 3;
    let dev_h = ch - ch / 8;
    let section = |x: i32, y: i32, h: i32| -> SectionLayout {
        SectionLayout {
            caption_pos: (client.x + x, client.y + y),
            textbox: IntRect {
                x: client.x + x,
                y: client.y + y + TITLE_LINE_HEIGHT,
                w: col_w,
                h: h - TITLE_LINE_HEIGHT,
            },
        }
    };
    let col2_top = dev_h / 2;
    let col3_first = dev_h / 3;
    let col3_second = (dev_h - col3_first) * 3 / 10;
    let sections = [
        section(0, 0, dev_h / 5),                       // Game Design
        section(0, dev_h / 5, dev_h - dev_h / 5),       // Engine and Tools
        section(col_w, 0, col2_top),                    // Scripting
        section(col_w, col2_top, dev_h - col2_top),     // Additional Art
        section(2 * col_w, 0, col3_first),              // Music
        section(2 * col_w, col3_first, col3_second),    // Voice
        section(2 * col_w, col3_first + col3_second, dev_h - col3_first - col3_second), // Web
    ];

    // LicenseWindow(caMain.GetAll()) followed by ComponentAligner(client,
    // 0, 10): the list consumes one fifth of the width and the TextWindow
    // receives the remainder (C4StartupAboutDlg.cpp:190-197,310-311).
    let license_window = IntRect {
        x: client.x,
        y: client.y,
        w: cw,
        h: dev_h,
    };
    let license_inner_y = license_window.y + 10;
    let license_inner_h = license_window.h - 20;
    let license_tabs_w = license_window.w / 5;
    let licenses = LicensePageLayout {
        window: license_window,
        tabs: IntRect {
            x: license_window.x,
            y: license_inner_y,
            w: license_tabs_w,
            h: license_inner_h,
        },
        text: IntRect {
            x: license_window.x + license_tabs_w,
            y: license_inner_y,
            w: license_window.w - license_tabs_w,
            h: license_inner_h,
        },
    };

    AboutLayout {
        client,
        title_anchor,
        trademark_anchor,
        buttons,
        sections,
        licenses,
    }
}

/// One credits entry: `(name, nick)`; either may be absent
/// (`PersonList::Entry`, C4StartupAboutDlg.cpp:40-44).
type Person = (Option<&'static str>, Option<&'static str>);

/// Formats one credits line like `DeveloperList::ToString` with
/// `with_color=true` (C4StartupAboutDlg.cpp:71-93): `Name<c f7f76f> (nick)</c>`,
/// nick-only `<c f7f76f>Nick</c>`, name-only plain.
pub fn credit_line(person: Person) -> String {
    match person {
        (Some(name), Some(nick)) => format!("{name}<c f7f76f> ({nick})</c>"),
        (Some(name), None) => name.to_string(),
        (None, Some(nick)) => format!("<c f7f76f>{nick}</c>"),
        (None, None) => String::new(),
    }
}

/// The seven credits sections in draw order: caption literal plus entries
/// (C4StartupAboutDlg.cpp:95-156,288-301; names are Latin-1 in C++, mapped
/// to the same Unicode scalars here).
pub const CREDITS_SECTIONS: [(&str, &[Person]); 7] = [
    ("Game Design", &[(Some("Matthes Bender"), Some("matthes"))]),
    (
        "Engine and Tools",
        &[
            (Some("Sven Eberhardt"), Some("Sven2")),
            (Some("Peter Wortmann"), Some("PeterW")),
            (Some("Günther Brammer"), Some("Günther")),
            (Some("Armin Burgmeier"), Some("Clonk-Karl")),
            (Some("Julian Raschke"), Some("survivor")),
            (Some("Alexander Post"), Some("qualle")),
            (Some("Jan Heberer"), Some("Jan")),
            (Some("Markus Mittendrein"), Some("Der Tod")),
            (Some("Dominik Bayerl"), Some("Kanibal")),
            (Some("George Tokmaji"), Some("Fulgen")),
            (Some("Martin Plicht"), Some("Mortimer")),
            (Some("Matthias Brehmer"), Some("Bratkartoffl")),
            (Some("Tim Kuhrt"), Some("TLK")),
        ],
    ),
    (
        "Scripting",
        &[
            (Some("Felix Wagner"), Some("Clonkonaut")),
            (Some("Richard Gerum"), Some("Randrian")),
            (Some("Markus Hoppe"), Some("Shamino")),
            (Some("David Dormagen"), Some("Zapper")),
            (Some("Florian Groß"), Some("flgr")),
            (Some("Tobias Zwick"), Some("Newton")),
            (Some("Bernhard Bonigl"), Some("boni")),
            (Some("Viktor Yuschuk"), Some("Viktor")),
            (None, Some("Raven")),
        ],
    ),
    (
        "Additional Art",
        &[
            (Some("Erik Nitzschke"), Some("DukeAufDune")),
            (Some("Merten Ehmig"), Some("pluto")),
            (Some("Matthias Rottländer"), Some("Matthi")),
            (Some("Christopher Reimann"), Some("Benzol")),
            (Some("Jonathan Veit"), Some("AniProGuy")),
            (Some("Arthur Möller"), Some("Aqua")),
            (Some("Tobias Zwick"), Some("Newton")),
            (None, Some("Raven")),
        ],
    ),
    (
        "Music",
        &[
            // "Hans-Christan" sic in C++ (C4StartupAboutDlg.cpp:141).
            (Some("Hans-Christan Kühl"), Some("HCK")),
            (Some("Sebastian Burkhart"), Some("hypo")),
            (Some("Florian Boos"), Some("Flobby")),
            (Some("Martin Strohmeier"), Some("K-Pone")),
        ],
    ),
    ("Voice", &[(Some("Klemens Köhring"), None)]),
    (
        "Web",
        &[
            (Some("Markus Wichitill"), Some("mawic")),
            (Some("Martin Schuster"), Some("knight_k")),
            (Some("Arne Bochem"), Some("ArneB")),
            (Some("Lukas Werling"), Some("Luchs")),
            (Some("Florian Graier"), Some("Nachtfalter")),
            (Some("Benedict Etzel"), Some("B_E")),
        ],
    ),
];

/// `FANPROJECTTEXT "   " TRADEMARKTEXT` (C4Version.h:21-22,
/// C4StartupAboutDlg.cpp:274).
pub const TRADEMARK_TEXT: &str = "LegacyClonk is a fan project based on Clonk Rage.   \
     'Clonk' is a registered trademark of Matthes Bender.";

/// Bottom-row button captions with `&` hotkey markers (LanguageUS.txt:
/// IDS_BTN_BACK, IDS_BTN_CHECKFORUPDATES, IDS_BTN_LICENSES).
pub const BUTTON_LABELS: [&str; 3] = ["Back", "Check for &updates", "&Licenses"];

/// One entry emitted into `generated/licenses.h` by the root CMake
/// `LICENSES` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AboutLicense {
    pub title: &'static str,
    pub license_title: &'static str,
    pub text: &'static str,
}

/// The baseline licenses compiled into LegacyClonk. Optional dependency
/// licenses are intentionally not fabricated when `deps/licenses.cmake` is
/// absent.
pub static ABOUT_LICENSES: [AboutLicense; 2] = [
    AboutLicense {
        title: "LegacyClonk",
        license_title: "ISC",
        text: include_str!("../../../../COPYING"),
    },
    AboutLicense {
        title: "Clonk Trademark",
        license_title: "",
        text: include_str!("../../../../TRADEMARK"),
    },
];

/// Graphics.c4g assets the dialog draws with.
pub struct AboutDlgAssets {
    /// `LoaderWatercave1.png` — fctAboutBG, stretched over the screen
    /// (C4Startup.cpp:50, C4StartupAboutDlg.cpp:356-359).
    pub background: ImageData,
    /// `GUIButton.png` — barButton 3-slice for released buttons
    /// (C4Gui.cpp:1089-1090).
    pub button: ImageData,
    /// `GUIScroll.png` — sfctScroll arrow/bar/pin slices
    /// (C4Gui.cpp:1098-1099,110-123).
    pub scroll: ImageData,
}

/// Visible page of `C4StartupAboutDlg` (`aboutPages`, cpp:286-312).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AboutPage {
    #[default]
    Credits,
    Licenses,
}

/// Observable callbacks from the About dialog. Page changes are applied to
/// [`AboutDlgState`] immediately; update checking deliberately remains an
/// external request (`C4StartupAboutDlg::OnUpdateBtn`, cpp:377-380).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AboutDlgAction {
    Back,
    CheckForUpdates,
    PageChanged(AboutPage),
    /// A user-driven license-list selection change. The app uses this to
    /// dispatch the classic ListBox selection sound.
    LicenseChanged(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AboutButton {
    Back,
    Update,
    Licenses,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AboutFocus {
    Button(AboutButton),
    LicenseTabs,
}

impl AboutButton {
    const ALL: [Self; 3] = [Self::Back, Self::Update, Self::Licenses];

    const fn index(self) -> usize {
        match self {
            Self::Back => 0,
            Self::Update => 1,
            Self::Licenses => 2,
        }
    }
}

/// Live controller/presentation state for the pixel-parity About dialog.
/// The C++ dialog has no initial focus. Buttons do not focus on click, while
/// the license ListBox does; matching releases and focused key releases invoke
/// buttons.
pub struct AboutDlgState {
    page: AboutPage,
    layout: Option<AboutLayout>,
    pointer_position: Option<GuiPoint>,
    hovered: Option<AboutButton>,
    pressed: Option<AboutButton>,
    focused: Option<AboutFocus>,
    credit_scroll_y: [i32; CREDITS_SECTIONS.len()],
    selected_license: Option<usize>,
    displayed_license: usize,
    license_tabs_scroll_y: i32,
    license_scroll_y: i32,
    license_max_scroll: [i32; ABOUT_LICENSES.len()],
}

impl Default for AboutDlgState {
    fn default() -> Self {
        Self::new()
    }
}

impl AboutDlgState {
    pub const fn new() -> Self {
        Self {
            page: AboutPage::Credits,
            layout: None,
            pointer_position: None,
            hovered: None,
            pressed: None,
            focused: None,
            credit_scroll_y: [0; CREDITS_SECTIONS.len()],
            selected_license: Some(0),
            displayed_license: 0,
            license_tabs_scroll_y: 0,
            license_scroll_y: 0,
            license_max_scroll: [0; ABOUT_LICENSES.len()],
        }
    }

    pub fn resize(&mut self, width: i32, height: i32, fonts: &ClonkFontSet) {
        let layout = about_layout(width.max(1), height.max(1));
        for (index, section) in layout.sections.iter().enumerate() {
            let metrics = credit_scroll_metrics(
                section,
                fonts,
                CREDITS_SECTIONS[index].1.len(),
            );
            self.credit_scroll_y[index] = metrics.clamp_offset(self.credit_scroll_y[index]);
        }
        self.license_tabs_scroll_y = license_tabs_scroll_metrics(&layout)
            .clamp_offset(self.license_tabs_scroll_y);
        for (index, license) in ABOUT_LICENSES.iter().enumerate() {
            self.license_max_scroll[index] =
                license_scroll_metrics(&layout, fonts, license).max_scroll;
        }
        self.license_scroll_y = self.license_scroll_y.clamp(
            0,
            self.license_max_scroll[self.displayed_license],
        );
        self.layout = Some(layout);
        self.hovered = self
            .pointer_position
            .and_then(|point| self.hit_test_button(point));
    }

    pub const fn current_page(&self) -> AboutPage {
        self.page
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn credit_scroll_offset(&self, section: usize) -> Option<i32> {
        self.credit_scroll_y.get(section).copied()
    }

    pub const fn selected_license_index(&self) -> Option<usize> {
        self.selected_license
    }

    pub const fn displayed_license_index(&self) -> usize {
        self.displayed_license
    }

    pub fn current_license(&self) -> &'static AboutLicense {
        &ABOUT_LICENSES[self.displayed_license]
    }

    pub const fn license_tabs_scroll_offset(&self) -> i32 {
        self.license_tabs_scroll_y
    }

    pub const fn license_scroll_offset(&self) -> i32 {
        self.license_scroll_y
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_test_button(point));
        if position.is_none() {
            self.pressed = None;
        }
    }

    pub fn pointer_left(&mut self) {
        self.set_pointer_position(None);
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<AboutDlgAction> {
        self.set_pointer_position(Some(position));
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<AboutDlgAction> {
        self.set_pointer_position(Some(position));
        if let Some(button) = self.hovered {
            self.pressed = Some(button);
            return Vec::new();
        }
        self.pressed = None;

        // ListBox selects on left-button down and takes focus on click
        // (C4GuiListBox.cpp:142-169; C4GuiListBox.h:54-56).
        if self.page == AboutPage::Licenses {
            if let Some(layout) = self.layout {
                if about_rect_contains(&license_tabs_viewport(&layout), position) {
                    self.focused = Some(AboutFocus::LicenseTabs);
                    let next = license_tab_at(
                        &layout,
                        position,
                        self.license_tabs_scroll_y,
                    );
                    if next != self.selected_license {
                        self.selected_license = next;
                        if let Some(index) = next {
                            self.set_displayed_license(index);
                            self.scroll_license_tab_into_view(index, &layout);
                            return vec![AboutDlgAction::LicenseChanged(index)];
                        }
                    }
                }
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<AboutDlgAction> {
        self.set_pointer_position(Some(position));
        let pressed = self.pressed.take();
        match (pressed, self.hovered) {
            (Some(button), Some(released)) if button == released => self.activate(button),
            _ => Vec::new(),
        }
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<AboutDlgAction> {
        match key {
            // Unlike the Back button, both dialog overrides leave directly,
            // including from the license page (C4StartupAboutDlg.h:36-39).
            KeyCode::Escape => {
                self.pressed = None;
                vec![AboutDlgAction::Back]
            }
            KeyCode::Tab => {
                self.pressed = None;
                self.advance_focus();
                Vec::new()
            }
            KeyCode::Up | KeyCode::Down
                if self.page == AboutPage::Licenses
                    && self.focused == Some(AboutFocus::LicenseTabs) =>
            {
                let next = match key {
                    KeyCode::Up => self
                        .selected_license
                        .map_or(ABOUT_LICENSES.len() - 1, |index| index.saturating_sub(1)),
                    KeyCode::Down => self
                        .selected_license
                        .map_or(0, |index| (index + 1).min(ABOUT_LICENSES.len() - 1)),
                    _ => unreachable!(),
                };
                if Some(next) == self.selected_license {
                    Vec::new()
                } else {
                    self.selected_license = Some(next);
                    self.set_displayed_license(next);
                    if let Some(layout) = self.layout {
                        self.scroll_license_tab_into_view(next, &layout);
                    }
                    vec![AboutDlgAction::LicenseChanged(next)]
                }
            }
            KeyCode::Enter => match self.focused_button() {
                Some(button) => {
                    self.pressed = Some(button);
                    Vec::new()
                }
                None => vec![AboutDlgAction::Back],
            },
            KeyCode::Space => {
                self.pressed = self.focused_button();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<AboutDlgAction> {
        if !matches!(key, KeyCode::Enter | KeyCode::Space) {
            return Vec::new();
        }
        self.pressed
            .take()
            .filter(|button| {
                self.focused == Some(AboutFocus::Button(*button)) && self.is_visible(*button)
            })
            .map(|button| self.activate(button))
            .unwrap_or_default()
    }

    /// Routes a native C++ wheel delta to the hovered `ScrollWindow`.
    /// Positive deltas scroll toward the top; callers should pass the same
    /// logical-pixel amount used by the engine (normally 60 per notch).
    pub fn handle_wheel(
        &mut self,
        position: GuiPoint,
        delta: i32,
        fonts: &ClonkFontSet,
    ) -> Vec<AboutDlgAction> {
        self.set_pointer_position(Some(position));
        let Some(layout) = self.layout else {
            return Vec::new();
        };
        if delta == 0 {
            return Vec::new();
        }

        match self.page {
            AboutPage::Credits => {
                for (index, section) in layout.sections.iter().enumerate() {
                    let viewport = credit_viewport(section);
                    let metrics = credit_scroll_metrics(
                        section,
                        fonts,
                        CREDITS_SECTIONS[index].1.len(),
                    );
                    if metrics.max_scroll > 0 && about_rect_contains(&viewport, position) {
                        self.credit_scroll_y[index] = metrics
                            .clamp_offset(self.credit_scroll_y[index].saturating_sub(delta));
                        break;
                    }
                }
            }
            AboutPage::Licenses => {
                let tabs_viewport = license_tabs_viewport(&layout);
                if about_rect_contains(&tabs_viewport, position) {
                    let metrics = license_tabs_scroll_metrics(&layout);
                    self.license_tabs_scroll_y = metrics
                        .clamp_offset(self.license_tabs_scroll_y.saturating_sub(delta));
                } else if about_rect_contains(&layout.licenses.text, position) {
                    let metrics = license_scroll_metrics(
                        &layout,
                        fonts,
                        &ABOUT_LICENSES[self.displayed_license],
                    );
                    self.license_scroll_y = metrics
                        .clamp_offset(self.license_scroll_y.saturating_sub(delta));
                }
            }
        }
        Vec::new()
    }

    fn activate(&mut self, button: AboutButton) -> Vec<AboutDlgAction> {
        match button {
            AboutButton::Back if self.page == AboutPage::Licenses => {
                self.page = AboutPage::Credits;
                self.hovered = self
                    .pointer_position
                    .and_then(|point| self.hit_test_button(point));
                vec![AboutDlgAction::PageChanged(self.page)]
            }
            AboutButton::Back => vec![AboutDlgAction::Back],
            AboutButton::Update => vec![AboutDlgAction::CheckForUpdates],
            AboutButton::Licenses if self.page == AboutPage::Credits => {
                self.page = AboutPage::Licenses;
                self.hovered = self
                    .pointer_position
                    .and_then(|point| self.hit_test_button(point));
                vec![AboutDlgAction::PageChanged(self.page)]
            }
            AboutButton::Licenses => Vec::new(),
        }
    }

    fn advance_focus(&mut self) {
        let mut visible: Vec<_> = AboutButton::ALL
            .iter()
            .copied()
            .filter(|button| self.is_visible(*button))
            .map(AboutFocus::Button)
            .collect();
        if self.page == AboutPage::Licenses {
            visible.push(AboutFocus::LicenseTabs);
        }
        if visible.is_empty() {
            self.focused = None;
            return;
        }
        self.focused = self
            .focused
            .and_then(|focused| visible.iter().position(|candidate| *candidate == focused))
            .map(|index| visible[(index + 1) % visible.len()])
            .or_else(|| visible.first().copied());
    }

    fn hit_test_button(&self, point: GuiPoint) -> Option<AboutButton> {
        let layout = self.layout.as_ref()?;
        AboutButton::ALL
            .iter()
            .copied()
            .filter(|button| self.is_visible(*button))
            .find(|button| about_rect_contains(&layout.buttons[button.index()], point))
    }

    const fn is_visible(&self, button: AboutButton) -> bool {
        !matches!((self.page, button), (AboutPage::Licenses, AboutButton::Licenses))
    }

    fn is_pressed(&self, button: AboutButton) -> bool {
        matches!(self.pressed, Some(pressed) if pressed == button)
    }

    fn focused_button(&self) -> Option<AboutButton> {
        match self.focused {
            Some(AboutFocus::Button(button)) if self.is_visible(button) => Some(button),
            _ => None,
        }
    }

    fn license_tabs_have_focus(&self) -> bool {
        self.page == AboutPage::Licenses && self.focused == Some(AboutFocus::LicenseTabs)
    }

    fn scroll_license_tab_into_view(&mut self, index: usize, layout: &AboutLayout) {
        let metrics = license_tabs_scroll_metrics(layout);
        let item_y = index as i32 * LICENSE_TAB_PITCH;
        let item_bottom = item_y + TEXT_LINE_HEIGHT;
        if self.license_tabs_scroll_y > item_y {
            self.license_tabs_scroll_y = item_y;
        } else if self.license_tabs_scroll_y + metrics.viewport_height < item_bottom {
            self.license_tabs_scroll_y = item_bottom - metrics.viewport_height;
        }
        self.license_tabs_scroll_y = metrics.clamp_offset(self.license_tabs_scroll_y);
    }

    fn set_displayed_license(&mut self, index: usize) {
        self.displayed_license = index;
        self.license_scroll_y = self
            .license_scroll_y
            .clamp(0, self.license_max_scroll[index]);
    }
}

fn about_rect_contains(rect: &IntRect, point: GuiPoint) -> bool {
    let (x, y) = (point.x.floor() as i32, point.y.floor() as i32);
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

/// C4GUI_Caption2FontClr / C4GUI_ButtonFontClr / FullscreenCaptionFontClr
/// 0xffffff00 after MakeColorReadableOnBlack forces alpha 0xff (C4Gui.h:54,
/// 56, 164; C4Gui.cpp:71-90).
const YELLOW: [u8; 4] = [255, 255, 0, 255];
/// C4GUI_MessageFontClr 0xffffffff (C4Gui.h:58).
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// MultilineLabel content height for `n` paragraph lines of `font`:
/// lh per line plus lh/3 before every line but the first
/// (MultilineLabel::UpdateHeight, C4GuiLabels.cpp:274-291).
fn content_height(font: &ClonkFont, lines: usize) -> i32 {
    let n = lines as i32;
    (font.line_height * n + (font.line_height / 3) * (n - 1)).max(5)
}

/// Logical scroll range of one C4GUI `ScrollWindow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AboutScrollMetrics {
    pub viewport_height: i32,
    pub content_height: i32,
    pub max_scroll: i32,
}

impl AboutScrollMetrics {
    pub fn clamp_offset(self, offset: i32) -> i32 {
        offset.clamp(0, self.max_scroll)
    }
}

fn credit_viewport(section: &SectionLayout) -> IntRect {
    // CustomMarginTextWindow<0,8,0,8>; its ScrollWindow always reserves the
    // 16px scrollbar width even while the auto-hidden bar is invisible.
    IntRect {
        x: section.textbox.x,
        y: section.textbox.y + 8,
        w: (section.textbox.w - SCROLL_BAR_WDT).max(0),
        h: (section.textbox.h - 16).max(0),
    }
}

fn credit_scroll_metrics(
    section: &SectionLayout,
    fonts: &ClonkFontSet,
    line_count: usize,
) -> AboutScrollMetrics {
    let viewport_height = credit_viewport(section).h;
    let content_height = content_height(&fonts.text, line_count);
    AboutScrollMetrics {
        viewport_height,
        content_height,
        max_scroll: (content_height - viewport_height).max(0),
    }
}

fn license_tabs_viewport(layout: &AboutLayout) -> IntRect {
    let tabs = layout.licenses.tabs;
    IntRect {
        x: tabs.x + 3,
        y: tabs.y + 3,
        w: (tabs.w - 6 - SCROLL_BAR_WDT).max(0),
        h: (tabs.h - 6).max(0),
    }
}

fn license_tabs_scrollbar(layout: &AboutLayout) -> IntRect {
    let tabs = layout.licenses.tabs;
    IntRect {
        x: tabs.x + tabs.w - 3 - SCROLL_BAR_WDT,
        y: tabs.y + 3,
        w: SCROLL_BAR_WDT,
        h: (tabs.h - 6).max(0),
    }
}

fn license_tabs_scroll_metrics(layout: &AboutLayout) -> AboutScrollMetrics {
    let viewport_height = license_tabs_viewport(layout).h;
    let content_height = ABOUT_LICENSES.len() as i32 * TEXT_LINE_HEIGHT
        + (ABOUT_LICENSES.len().saturating_sub(1)) as i32 * LIST_ROW_SPACING;
    AboutScrollMetrics {
        viewport_height,
        content_height,
        max_scroll: (content_height - viewport_height).max(0),
    }
}

fn license_tab_at(layout: &AboutLayout, point: GuiPoint, scroll_y: i32) -> Option<usize> {
    let viewport = license_tabs_viewport(layout);
    if !about_rect_contains(&viewport, point) {
        return None;
    }
    let y = point.y.floor() as i32;
    let metrics = license_tabs_scroll_metrics(layout);
    let content_y = y - viewport.y + metrics.clamp_offset(scroll_y);
    let index = (content_y / LICENSE_TAB_PITCH) as usize;
    let within_row = content_y % LICENSE_TAB_PITCH < TEXT_LINE_HEIGHT;
    (within_row && index < ABOUT_LICENSES.len()).then_some(index)
}

fn license_text_viewport(layout: &AboutLayout) -> IntRect {
    let text = layout.licenses.text;
    // TextWindow margins L10/T8/R5/B8, followed by the ScrollWindow's
    // reserved scrollbar column.
    IntRect {
        x: text.x + 10,
        y: text.y + 8,
        w: (text.w - 15 - SCROLL_BAR_WDT).max(0),
        h: (text.h - 16).max(0),
    }
}

fn license_text_scrollbar(layout: &AboutLayout) -> IntRect {
    let text = layout.licenses.text;
    IntRect {
        x: text.x + text.w - 5 - SCROLL_BAR_WDT,
        y: text.y + 8,
        w: SCROLL_BAR_WDT,
        h: (text.h - 16).max(0),
    }
}

fn license_display_title(license: &AboutLicense) -> String {
    if license.license_title.is_empty() {
        license.title.to_string()
    } else {
        format!("{} ({})", license.title, license.license_title)
    }
}

struct LicenseTextLine<'a> {
    text: String,
    font: &'a ClonkFont,
    color: [u8; 4],
    new_paragraph: bool,
}

fn append_license_line<'a>(
    output: &mut Vec<LicenseTextLine<'a>>,
    text: &str,
    font: &'a ClonkFont,
    color: [u8; 4],
    width: i32,
) {
    // C4LogBuffer drops empty messages. Wrapped continuation lines are not
    // new paragraphs; the first physical line from each AddTextLine is.
    if text.is_empty() {
        return;
    }
    for (index, physical) in break_message(font, text, width.max(1))
        .split('\n')
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        output.push(LicenseTextLine {
            text: physical.to_string(),
            font,
            color,
            new_paragraph: index == 0,
        });
    }
}

fn license_text_lines<'a>(
    layout: &AboutLayout,
    fonts: &'a ClonkFontSet,
    license: &AboutLicense,
) -> Vec<LicenseTextLine<'a>> {
    let mut lines = Vec::new();
    let width = license_text_viewport(layout).w;
    append_license_line(
        &mut lines,
        &license_display_title(license),
        &fonts.title,
        YELLOW,
        width,
    );
    // std::getline(..., '\n') feeds every source line to AddTextLine; the
    // C4LogBuffer itself ignores empty strings.
    for source_line in license.text.split('\n') {
        append_license_line(&mut lines, source_line, &fonts.text, WHITE, width);
    }
    lines
}

fn license_lines_height(lines: &[LicenseTextLine<'_>]) -> i32 {
    let mut height = 0;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 && line.new_paragraph {
            height += line.font.line_height / 3;
        }
        height += line.font.line_height;
    }
    height.max(5)
}

pub fn license_scroll_metrics(
    layout: &AboutLayout,
    fonts: &ClonkFontSet,
    license: &AboutLicense,
) -> AboutScrollMetrics {
    let viewport_height = license_text_viewport(layout).h;
    let content_height = license_lines_height(&license_text_lines(layout, fonts, license));
    AboutScrollMetrics {
        viewport_height,
        content_height,
        max_scroll: (content_height - viewport_height).max(0),
    }
}

/// Draws a vertical scrollbar from the GUIScroll slices. DrawVBar tiles the
/// bar to `h-5` between the arrow caps; the pin maps window scroll range to
/// `h - 2*arrow - thumb` (C4GuiContainers.cpp:343-475).
/// Slices per ScrollBarFacets::Set (C4Gui.cpp:110-123).
fn draw_scrollbar(
    surface: &mut Surface,
    x: i32,
    y: i32,
    h: i32,
    scroll_y: i32,
    max_scroll: i32,
    scroll: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_strip(surface, x, y, scroll, 0, 0, 16, 16, gamma);
    let mut iy = 16;
    while iy < h - 5 {
        let tile_h = 16.min(h - 5 - iy).max(0) as u32;
        crate::draw_image_strip(surface, x, y + iy, scroll, 0, 16, 16, tile_h, gamma);
        iy += 16;
    }
    crate::draw_image_strip(surface, x, y + h - 16, scroll, 0, 32, 16, 16, gamma);
    if max_scroll > 0 {
        let max_pin = (h - 3 * SCROLL_BAR_WDT).max(0);
        let pin_y = y + SCROLL_BAR_WDT
            + max_pin * scroll_y.clamp(0, max_scroll) / max_scroll;
        crate::draw_image_strip(surface, x, pin_y, scroll, 16, 16, 16, 16, gamma);
    }
}

fn draw_credits_page(
    surface: &mut Surface,
    assets: &AboutDlgAssets,
    fonts: &ClonkFontSet,
    state: &AboutDlgState,
    layout: &AboutLayout,
    gamma: Option<&GammaRamp>,
) {
    for (section_index, (section, (caption, people))) in layout
        .sections
        .iter()
        .zip(CREDITS_SECTIONS)
        .enumerate()
    {
        fonts.caption.draw_with_gamma(
            surface,
            section.caption_pos.0,
            section.caption_pos.1,
            caption,
            YELLOW,
            TextAlign::Left,
            true,
            gamma,
        );

        let viewport = credit_viewport(section);
        let metrics = credit_scroll_metrics(section, fonts, people.len());
        let scroll_y = metrics.clamp_offset(state.credit_scroll_y[section_index]);
        let mut y = viewport.y - scroll_y;
        for (index, person) in people.iter().enumerate() {
            if index > 0 {
                y += fonts.text.line_height / 3;
            }
            if y < viewport.y + viewport.h && y + fonts.text.line_height > viewport.y {
                draw_clipped_text(
                    surface,
                    &fonts.text,
                    viewport.x,
                    y,
                    &credit_line(*person),
                    WHITE,
                    TextAlign::Left,
                    gamma,
                    viewport,
                );
            }
            y += fonts.text.line_height;
        }

        // SetDecoration(..., fAutoScroll=true) hides only the bar, not the
        // scrollbar column reserved by ScrollWindow.
        if metrics.max_scroll > 0 {
            draw_scrollbar(
                surface,
                section.textbox.x + section.textbox.w - SCROLL_BAR_WDT,
                viewport.y,
                viewport.h,
                scroll_y,
                metrics.max_scroll,
                &assets.scroll,
                gamma,
            );
        }
    }
}

fn draw_license_page(
    surface: &mut Surface,
    assets: &AboutDlgAssets,
    fonts: &ClonkFontSet,
    state: &AboutDlgState,
    layout: &AboutLayout,
    gamma: Option<&GammaRamp>,
) {
    // LicenseWindow child 1: ListBox background, selected-row fill, rows,
    // then its always-visible default scrollbar.
    let tabs = layout.licenses.tabs;
    draw_engine_box(
        surface,
        tabs.x,
        tabs.y,
        tabs.x + tabs.w - 1,
        tabs.y + tabs.h - 1,
        0x7f00_0000,
        gamma,
    );
    let tab_viewport = license_tabs_viewport(layout);
    let tab_metrics = license_tabs_scroll_metrics(layout);
    let tab_scroll_y = tab_metrics.clamp_offset(state.license_tabs_scroll_y);
    if let Some(selected_license) = state.selected_license {
        let selected_y =
            tab_viewport.y + selected_license as i32 * LICENSE_TAB_PITCH - tab_scroll_y;
        let clipped_y = selected_y.max(tab_viewport.y);
        let clipped_bottom = (selected_y + TEXT_LINE_HEIGHT)
            .min(tab_viewport.y + tab_viewport.h);
        if clipped_y < clipped_bottom {
            let selection_color = if state.license_tabs_have_focus() {
                0xafaf_0000
            } else {
                0xaf7f_7f7f
            };
            draw_engine_box(
                surface,
                tab_viewport.x,
                clipped_y,
                tab_viewport.x + tab_viewport.w - 1,
                clipped_bottom - 1,
                selection_color,
                gamma,
            );
        }
    }
    for (index, license) in ABOUT_LICENSES.iter().enumerate() {
        let y = tab_viewport.y + index as i32 * LICENSE_TAB_PITCH - tab_scroll_y;
        if y >= tab_viewport.y + tab_viewport.h {
            break;
        }
        if y + TEXT_LINE_HEIGHT > tab_viewport.y {
            draw_clipped_text(
                surface,
                &fonts.text,
                tab_viewport.x,
                y,
                license.title,
                WHITE,
                TextAlign::Left,
                gamma,
                tab_viewport,
            );
        }
    }
    let tabs_bar = license_tabs_scrollbar(layout);
    draw_scrollbar(
        surface,
        tabs_bar.x,
        tabs_bar.y,
        tabs_bar.h,
        tab_scroll_y,
        tab_metrics.max_scroll,
        &assets.scroll,
        gamma,
    );

    // LicenseWindow child 2: standard TextWindow background/frame, wrapped
    // MultilineLabel, then its non-auto-hidden scrollbar.
    let text = layout.licenses.text;
    draw_engine_box(
        surface,
        text.x,
        text.y,
        text.x + text.w - 1,
        text.y + text.h - 1,
        0x7f00_0000,
        gamma,
    );
    draw_3d_frame(surface, text, gamma);

    let license = &ABOUT_LICENSES[state.displayed_license];
    let lines = license_text_lines(layout, fonts, license);
    let viewport = license_text_viewport(layout);
    let metrics = AboutScrollMetrics {
        viewport_height: viewport.h,
        content_height: license_lines_height(&lines),
        max_scroll: (license_lines_height(&lines) - viewport.h).max(0),
    };
    let scroll_y = metrics.clamp_offset(state.license_scroll_y);
    let mut y = viewport.y - scroll_y;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 && line.new_paragraph {
            y += line.font.line_height / 3;
        }
        if y < viewport.y + viewport.h && y + line.font.line_height > viewport.y {
            draw_clipped_text(
                surface,
                line.font,
                viewport.x,
                y,
                &line.text,
                line.color,
                TextAlign::Left,
                gamma,
                viewport,
            );
        }
        y += line.font.line_height;
    }
    let text_bar = license_text_scrollbar(layout);
    draw_scrollbar(
        surface,
        text_bar.x,
        text_bar.y,
        text_bar.h,
        scroll_y,
        metrics.max_scroll,
        &assets.scroll,
        gamma,
    );
}

/// Renders the live state of C4StartupAboutDlg in the C++ draw order
/// (about.md §"Exact draw order").
pub struct AboutDlgScreen;

impl AboutDlgScreen {
    /// Source-compatible first-shown renderer. New callers with live input
    /// state should use [`Self::render_state`].
    pub fn render(
        surface: &mut Surface,
        assets: &AboutDlgAssets,
        fonts: &ClonkFontSet,
        gamma: Option<&GammaRamp>,
    ) {
        Self::render_state(surface, assets, fonts, &AboutDlgState::default(), gamma);
    }

    pub fn render_state(
        surface: &mut Surface,
        assets: &AboutDlgAssets,
        fonts: &ClonkFontSet,
        state: &AboutDlgState,
        gamma: Option<&GammaRamp>,
    ) {
        let (w, h) = (surface.width() as i32, surface.height() as i32);
        let layout = about_layout(w, h);

        // 1. fctAboutBG stretched over screen bounds expanded by 1px
        // (DrawBackground, C4GuiDialogs.cpp:878-887).
        let bg_rect = GuiRect::new(-1.0, -1.0, (w + 2) as f32, (h + 2) as f32);
        crate::draw_image_bilinear(surface, &bg_rect, &assets.background, gamma);

        // 2. Title label "&About": TitleFont, ACenter, yellow, hotkey markup
        // (C4GuiDialogs.cpp:834-850; C4StartupAboutDlg.cpp:262).
        let (title, _) = expand_hotkey_markup("&About");
        fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            &title,
            YELLOW,
            TextAlign::Center,
            true,
            gamma,
        );

        // 3. Trademark label: MiniFont, ARight, white
        // (C4StartupAboutDlg.cpp:274-275).
        fonts.mini.draw_with_gamma(
            surface,
            layout.trademark_anchor.0,
            layout.trademark_anchor.1,
            TRADEMARK_TEXT,
            WHITE,
            TextAlign::Right,
            true,
            gamma,
        );

        // 4-6. Back / Update / Licenses buttons (Button::DrawElement,
        // C4GuiButton.cpp:81-109): barButton 3-slice, then the caption in the
        // largest GUI font fitting Hgt-2 (CaptionFont at 32px), yellow,
        // centered at ((x0+x1)/2, (y0+y1-lh)/2).
        for (index, (rect, label)) in layout.buttons.iter().zip(BUTTON_LABELS).enumerate() {
            let button = AboutButton::ALL[index];
            if !state.is_visible(button) {
                continue;
            }
            let bar = GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
            draw_bar(surface, &bar, &assets.button, gamma);
            let font = fonts.button_font(rect.h);
            let (caption, _) = expand_hotkey_markup(label);
            let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
            let pressed_offset = i32::from(state.is_pressed(button));
            font.draw_with_gamma(
                surface,
                (rect.x + x1) / 2 + pressed_offset,
                (rect.y + y1 - font.line_height) / 2 + pressed_offset,
                &caption,
                YELLOW,
                TextAlign::Center,
                true,
                gamma,
            );
        }

        match state.current_page() {
            AboutPage::Credits => {
                // 7+. Caption labels and undecorated auto-scrolling
                // TextWindows (C4StartupAboutDlg.cpp:325-350).
                draw_credits_page(surface, assets, fonts, state, &layout, gamma);
            }
            AboutPage::Licenses => {
                draw_license_page(surface, assets, fonts, state, &layout, gamma);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pins the spec table for 1280x720 (rust/target/parity-specs/about.md;
    // C4StartupAboutDlg.cpp:262-301 aligner math verified against the C++ F9
    // reference capture).
    #[test]
    fn layout_matches_cpp_about_dlg_at_1280x720() {
        let layout = about_layout(1280, 720);

        assert_eq!(
            (layout.client.x, layout.client.y, layout.client.w, layout.client.h),
            (25, 69, 1230, 632)
        );
        assert_eq!(layout.title_anchor, (640, 8));
        assert_eq!(layout.trademark_anchor, (1255, 683));

        let rects: Vec<_> = layout
            .buttons
            .iter()
            .map(|r| (r.x, r.y, r.w, r.h))
            .collect();
        assert_eq!(
            rects,
            vec![(76, 636, 307, 32), (639, 636, 307, 32), (946, 636, 307, 32)]
        );

        let sections: Vec<_> = layout
            .sections
            .iter()
            .map(|s| {
                (
                    s.caption_pos,
                    (s.textbox.x, s.textbox.y, s.textbox.w, s.textbox.h),
                )
            })
            .collect();
        assert_eq!(
            sections,
            vec![
                ((25, 69), (25, 103, 410, 76)),    // Game Design
                ((25, 179), (25, 213, 410, 409)),  // Engine and Tools
                ((435, 69), (435, 103, 410, 242)), // Scripting
                ((435, 345), (435, 379, 410, 243)), // Additional Art
                ((845, 69), (845, 103, 410, 150)), // Music
                ((845, 253), (845, 287, 410, 76)), // Voice
                ((845, 363), (845, 397, 410, 225)), // Web
            ]
        );
        assert_eq!(
            (
                layout.licenses.window.x,
                layout.licenses.window.y,
                layout.licenses.window.w,
                layout.licenses.window.h,
            ),
            (25, 69, 1230, 553)
        );
        assert_eq!(
            (
                layout.licenses.tabs.x,
                layout.licenses.tabs.y,
                layout.licenses.tabs.w,
                layout.licenses.tabs.h,
            ),
            (25, 79, 246, 533)
        );
        assert_eq!(
            (
                layout.licenses.text.x,
                layout.licenses.text.y,
                layout.licenses.text.w,
                layout.licenses.text.h,
            ),
            (271, 79, 984, 533)
        );
    }

    #[test]
    fn baseline_licenses_match_generated_cpp_inputs() {
        assert_eq!(ABOUT_LICENSES.len(), 2);
        assert_eq!(license_display_title(&ABOUT_LICENSES[0]), "LegacyClonk (ISC)");
        assert_eq!(license_display_title(&ABOUT_LICENSES[1]), "Clonk Trademark");
        assert_eq!(ABOUT_LICENSES[0].text, include_str!("../../../../COPYING"));
        assert_eq!(ABOUT_LICENSES[1].text, include_str!("../../../../TRADEMARK"));
        assert!(ABOUT_LICENSES[0].text.contains("Permission to use, copy, modify"));
        assert!(ABOUT_LICENSES[1].text.contains("registered trademark"));
    }

    // DeveloperList::ToString with color (C4StartupAboutDlg.cpp:71-93):
    // the space before the paren sits INSIDE the color tag.
    #[test]
    fn credit_line_formats_name_nick_combinations() {
        assert_eq!(
            credit_line((Some("Matthes Bender"), Some("matthes"))),
            "Matthes Bender<c f7f76f> (matthes)</c>"
        );
        assert_eq!(credit_line((None, Some("Raven"))), "<c f7f76f>Raven</c>");
        assert_eq!(credit_line((Some("Klemens Köhring"), None)), "Klemens Köhring");
    }

    // Scrollbar auto-hide on first show (about.md overflow table): only
    // Scripting (9 lines in a 226px viewport) overflows. Content height =
    // 22n + 7(n-1) (MultilineLabel::UpdateHeight, C4GuiLabels.cpp:274-291).
    #[test]
    fn only_scripting_section_shows_a_scrollbar_at_1280x720() {
        let font = ClonkFont::new(22);
        let layout = about_layout(1280, 720);
        let overflowing: Vec<&str> = layout
            .sections
            .iter()
            .zip(CREDITS_SECTIONS)
            .filter(|(section, (_, people))| {
                content_height(&font, people.len()) > section.textbox.h - 16
            })
            .map(|(_, (caption, _))| caption)
            .collect();
        assert_eq!(overflowing, vec!["Scripting"]);
        // Scripting: content 254 > viewport 226 (9 rows; row 9 clipped).
        assert_eq!(content_height(&font, 9), 254);
    }

    #[test]
    fn credits_wheel_scrolls_only_the_hovered_overflowing_textwindow() {
        let fonts = crate::test_support::endeavour_font_set();
        let layout = about_layout(1280, 720);
        let mut state = AboutDlgState::new();
        state.resize(1280, 720, &fonts);
        let scripting = credit_viewport(&layout.sections[2]);
        let point = GuiPoint::new((scripting.x + 1) as f32, (scripting.y + 1) as f32);

        assert!(state.handle_wheel(point, -60, &fonts).is_empty());
        assert_eq!(state.credit_scroll_offset(2), Some(28));
        assert_eq!(state.credit_scroll_offset(1), Some(0));
        assert!(state.handle_wheel(point, 60, &fonts).is_empty());
        assert_eq!(state.credit_scroll_offset(2), Some(0));
    }

    // Buttons invoke callbacks only after a matching down/up
    // (C4GuiButton.cpp:130-154). Update checking remains an external action
    // (`C4StartupAboutDlg::OnUpdateBtn`, cpp:377-380).
    #[test]
    fn live_state_hits_exact_about_buttons_and_emits_update_request() {
        let mut state = AboutDlgState::default();
        state.resize(1280, 720, &crate::test_support::endeavour_font_set());
        let layout = about_layout(1280, 720);
        let update = layout.buttons[1];
        let point = crate::GuiPoint::new(update.x as f32, update.y as f32);

        assert!(state.handle_pointer_down(point).is_empty());
        assert_eq!(
            state.handle_pointer_up(point),
            vec![AboutDlgAction::CheckForUpdates]
        );
        assert_eq!(
            state.handle_key_down(crate::KeyCode::Enter),
            vec![AboutDlgAction::Back],
            "Button::IsFocusOnClick is false, so the pointer click must not focus Update"
        );

        // x2 is the Licenses button's x0 at 1280px, so step above the row to
        // test the Update rectangle's half-open edge without hitting it.
        let outside = crate::GuiPoint::new(
            (update.x + update.w) as f32,
            (update.y - 1) as f32,
        );
        assert!(state.handle_pointer_down(outside).is_empty());
        assert!(state.handle_pointer_up(outside).is_empty());
    }

    // OnAdvanceButton shows page 1 and hides itself; the Back *button* walks
    // to page 0 before leaving (C4StartupAboutDlg.h:39-40, cpp:361-375).
    #[test]
    fn live_state_licenses_and_back_follow_cpp_page_stack() {
        let mut state = AboutDlgState::default();
        state.resize(1280, 720, &crate::test_support::endeavour_font_set());
        let layout = about_layout(1280, 720);
        let licenses = layout.buttons[2];
        let licenses_point = crate::GuiPoint::new(licenses.x as f32, licenses.y as f32);

        state.handle_pointer_down(licenses_point);
        assert_eq!(
            state.handle_pointer_up(licenses_point),
            vec![AboutDlgAction::PageChanged(AboutPage::Licenses)]
        );
        assert_eq!(state.current_page(), AboutPage::Licenses);
        assert!(state.handle_pointer_down(licenses_point).is_empty());
        assert!(state.handle_pointer_up(licenses_point).is_empty());

        let back = layout.buttons[0];
        let back_point = crate::GuiPoint::new(back.x as f32, back.y as f32);
        state.handle_pointer_down(back_point);
        assert_eq!(
            state.handle_pointer_up(back_point),
            vec![AboutDlgAction::PageChanged(AboutPage::Credits)]
        );
        state.handle_pointer_down(back_point);
        assert_eq!(state.handle_pointer_up(back_point), vec![AboutDlgAction::Back]);
    }

    #[test]
    fn license_list_pointer_and_keyboard_selection_follow_listbox_rules() {
        let mut state = AboutDlgState::new();
        let fonts = crate::test_support::endeavour_font_set();
        state.resize(1280, 720, &fonts);
        let layout = about_layout(1280, 720);
        let advance = layout.buttons[2];
        let advance_point = GuiPoint::new(advance.x as f32, advance.y as f32);
        state.handle_pointer_down(advance_point);
        state.handle_pointer_up(advance_point);

        let tabs = license_tabs_viewport(&layout);
        let second = GuiPoint::new(
            (tabs.x + 1) as f32,
            (tabs.y + TEXT_LINE_HEIGHT + 1) as f32,
        );
        assert_eq!(
            state.handle_pointer_down(second),
            vec![AboutDlgAction::LicenseChanged(1)]
        );
        assert_eq!(state.selected_license_index(), Some(1));
        assert_eq!(state.current_license().title, "Clonk Trademark");

        // ListBox forwards selection only through its client ScrollWindow.
        // Its 3px frame and the adjacent scrollbar preserve the selection.
        let border = GuiPoint::new(layout.licenses.tabs.x as f32, tabs.y as f32);
        assert!(state.handle_pointer_down(border).is_empty());
        assert_eq!(state.selected_license_index(), Some(1));
        let scrollbar = license_tabs_scrollbar(&layout);
        let scrollbar_point = GuiPoint::new(
            (scrollbar.x + 1) as f32,
            (scrollbar.y + 1) as f32,
        );
        assert!(state.handle_pointer_down(scrollbar_point).is_empty());
        assert_eq!(state.selected_license_index(), Some(1));

        assert_eq!(
            state.handle_key_down(KeyCode::Up),
            vec![AboutDlgAction::LicenseChanged(0)]
        );
        assert!(state.handle_key_down(KeyCode::Up).is_empty());
        assert_eq!(state.selected_license_index(), Some(0));

        // The 1px C4GUI_DefaultListSpacing gap is not part of either Label.
        let gap = GuiPoint::new(
            (tabs.x + 1) as f32,
            (tabs.y + TEXT_LINE_HEIGHT) as f32,
        );
        assert!(state.handle_pointer_down(gap).is_empty());
        assert_eq!(state.selected_license_index(), None);
        assert_eq!(state.displayed_license_index(), 0);
        assert_eq!(
            state.handle_key_down(KeyCode::Down),
            vec![AboutDlgAction::LicenseChanged(0)]
        );
    }

    #[test]
    fn license_text_wrap_metrics_and_wheel_clamp_on_narrow_screens() {
        let fonts = crate::test_support::endeavour_font_set();
        let layout = about_layout(320, 240);
        let metrics = license_scroll_metrics(&layout, &fonts, &ABOUT_LICENSES[0]);
        assert!(metrics.content_height > metrics.viewport_height);
        assert!(metrics.max_scroll > 0);

        let mut state = AboutDlgState::new();
        state.resize(320, 240, &fonts);
        let advance = layout.buttons[2];
        let advance_point = GuiPoint::new(advance.x as f32, advance.y as f32);
        state.handle_pointer_down(advance_point);
        state.handle_pointer_up(advance_point);
        let viewport = license_text_viewport(&layout);
        let text_point = GuiPoint::new((viewport.x + 1) as f32, (viewport.y + 1) as f32);

        state.handle_wheel(text_point, -i32::MAX, &fonts);
        assert_eq!(state.license_scroll_offset(), metrics.max_scroll);

        let tabs = license_tabs_viewport(&layout);
        let second = GuiPoint::new(
            (tabs.x + 1) as f32,
            (tabs.y + LICENSE_TAB_PITCH + 1) as f32,
        );
        let shorter = license_scroll_metrics(&layout, &fonts, &ABOUT_LICENSES[1]);
        assert_eq!(
            state.handle_pointer_down(second),
            vec![AboutDlgAction::LicenseChanged(1)]
        );
        assert_eq!(
            state.license_scroll_offset(),
            shorter.clamp_offset(metrics.max_scroll),
            "changing content clamps the stored ScrollWindow offset immediately"
        );
        let before_up = state.license_scroll_offset();
        state.handle_wheel(text_point, 60, &fonts);
        assert_eq!(
            state.license_scroll_offset(),
            before_up.saturating_sub(60),
            "the first upward wheel must not be consumed by a stale offset"
        );

        state.handle_wheel(text_point, -i32::MAX, &fonts);
        let wide_layout = about_layout(1280, 720);
        let resized = license_scroll_metrics(&wide_layout, &fonts, &ABOUT_LICENSES[1]);
        state.resize(1280, 720, &fonts);
        assert_eq!(
            state.license_scroll_offset(),
            resized.clamp_offset(shorter.max_scroll),
            "resizing clamps the stored ScrollWindow offset to its new range"
        );

        state.resize(320, 240, &fonts);
        state.handle_wheel(text_point, i32::MAX, &fonts);
        assert_eq!(state.license_scroll_offset(), 0);
    }

    #[test]
    fn license_list_wheel_scrolls_its_second_scrollwindow_at_tiny_heights() {
        let fonts = crate::test_support::endeavour_font_set();
        let layout = about_layout(320, 100);
        let metrics = license_tabs_scroll_metrics(&layout);
        assert_eq!(metrics.content_height, 45);
        assert!(metrics.max_scroll > 0);

        let mut state = AboutDlgState::new();
        state.resize(320, 100, &fonts);
        state.page = AboutPage::Licenses;
        let viewport = license_tabs_viewport(&layout);
        let point = GuiPoint::new((viewport.x + 1) as f32, (viewport.y + 1) as f32);
        state.handle_wheel(point, -60, &fonts);
        assert_eq!(state.license_tabs_scroll_offset(), metrics.max_scroll);
        state.handle_wheel(point, 60, &fonts);
        assert_eq!(state.license_tabs_scroll_offset(), 0);
    }

    // No control has focus on first show. Therefore dialog Enter leaves;
    // Tab selects Back, Update, Licenses in add order, and a focused button
    // activates on key-up (C4GuiDialogs.cpp:616-646; C4GuiButton.cpp:112-128).
    #[test]
    fn live_state_keyboard_focus_and_activation_match_about_dialog() {
        let mut state = AboutDlgState::default();
        state.resize(
            1280,
            720,
            &crate::test_support::endeavour_font_set(),
        );
        assert_eq!(state.handle_key_down(crate::KeyCode::Enter), vec![AboutDlgAction::Back]);

        assert!(state.handle_key_down(crate::KeyCode::Tab).is_empty()); // Back
        assert!(state.handle_key_down(crate::KeyCode::Tab).is_empty()); // Update
        assert!(state.handle_key_down(crate::KeyCode::Space).is_empty());
        assert_eq!(
            state.handle_key_up(crate::KeyCode::Space),
            vec![AboutDlgAction::CheckForUpdates]
        );

        assert!(state.handle_key_down(crate::KeyCode::Tab).is_empty()); // Licenses
        assert!(state.handle_key_down(crate::KeyCode::Enter).is_empty());
        assert_eq!(
            state.handle_key_up(crate::KeyCode::Enter),
            vec![AboutDlgAction::PageChanged(AboutPage::Licenses)]
        );
        assert_eq!(state.handle_key_down(crate::KeyCode::Escape), vec![AboutDlgAction::Back]);
    }

    // Renders the dialog at 1280x720 and dumps a PPM for the manual diff
    // against the C++ F9 reference (build/Screenshots/ref-about.png). CI has
    // no reference image, so this only produces the artifact.
    #[test]
    fn render_writes_reference_artifact() {
        use crate::test_support::{
            endeavour_font_set, load_graphics_png, standard_gamma, write_ppm,
        };
        use lc_graphics::PixelFormat;

        let assets = AboutDlgAssets {
            background: load_graphics_png("LoaderWatercave1.png"),
            button: load_graphics_png("GUIButton.png"),
            scroll: load_graphics_png("GUIScroll.png"),
        };
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
        AboutDlgScreen::render(&mut surface, &assets, &fonts, Some(standard_gamma()));
        // Final whole-surface gamma pass, mirroring the app's
        // render_startup_frame (lc-app main.rs render_startup_frame).
        standard_gamma().apply_to_surface(&mut surface);

        std::fs::create_dir_all("/tmp/menu-parity-about").expect("create artifact dir");
        write_ppm(&surface, "/tmp/menu-parity-about/out.ppm");
    }
}
