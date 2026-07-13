//! Pixel-parity renderer for one C++ startup dialog (see
//! `rust/target/parity-specs/`). Implemented against the engine's F9
//! reference captures; owned by its implementation agent.
//!
//! Dialog: `C4StartupAboutDlg` (C4StartupAboutDlg.cpp). All geometry mirrors
//! the C++ integer math exactly; see `rust/target/parity-specs/about.md`.

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
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

    AboutLayout {
        client,
        title_anchor,
        trademark_anchor,
        buttons,
        sections,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AboutButton {
    Back,
    Update,
    Licenses,
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
/// The C++ dialog has no initial focus; pointer presses retain keyboard focus,
/// while matching pointer releases and focused key releases invoke buttons.
pub struct AboutDlgState {
    page: AboutPage,
    layout: Option<AboutLayout>,
    pointer_position: Option<GuiPoint>,
    hovered: Option<AboutButton>,
    pressed: Option<AboutButton>,
    focused: Option<AboutButton>,
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
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.layout = Some(about_layout(width.max(1), height.max(1)));
        self.hovered = self.pointer_position.and_then(|point| self.hit_test(point));
    }

    pub const fn current_page(&self) -> AboutPage {
        self.page
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
        self.hovered = position.and_then(|point| self.hit_test(point));
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
        self.pressed = self.hovered;
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
            KeyCode::Enter => match self.focused.filter(|button| self.is_visible(*button)) {
                Some(button) => {
                    self.pressed = Some(button);
                    Vec::new()
                }
                None => vec![AboutDlgAction::Back],
            },
            KeyCode::Space => {
                self.pressed = self.focused.filter(|button| self.is_visible(*button));
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
            .filter(|button| self.focused == Some(*button) && self.is_visible(*button))
            .map(|button| self.activate(button))
            .unwrap_or_default()
    }

    fn activate(&mut self, button: AboutButton) -> Vec<AboutDlgAction> {
        match button {
            AboutButton::Back if self.page == AboutPage::Licenses => {
                self.page = AboutPage::Credits;
                self.hovered = self.pointer_position.and_then(|point| self.hit_test(point));
                vec![AboutDlgAction::PageChanged(self.page)]
            }
            AboutButton::Back => vec![AboutDlgAction::Back],
            AboutButton::Update => vec![AboutDlgAction::CheckForUpdates],
            AboutButton::Licenses if self.page == AboutPage::Credits => {
                self.page = AboutPage::Licenses;
                self.hovered = self.pointer_position.and_then(|point| self.hit_test(point));
                vec![AboutDlgAction::PageChanged(self.page)]
            }
            AboutButton::Licenses => Vec::new(),
        }
    }

    fn advance_focus(&mut self) {
        let visible: Vec<_> = AboutButton::ALL
            .iter()
            .copied()
            .filter(|button| self.is_visible(*button))
            .collect();
        if visible.is_empty() {
            self.focused = None;
            return;
        }
        self.focused = self
            .focused
            .and_then(|focused| visible.iter().position(|button| *button == focused))
            .map(|index| visible[(index + 1) % visible.len()])
            .or_else(|| visible.first().copied());
    }

    fn hit_test(&self, point: GuiPoint) -> Option<AboutButton> {
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

/// Draws a vertical scrollbar at scroll position 0 from the GUIScroll
/// slices: DrawVBar tiles the bar to `h-5` between the arrow caps
/// (C4GuiContainers.cpp:446-475), then the pin at `y + 16 + iScrollPos`.
/// Slices per ScrollBarFacets::Set (C4Gui.cpp:110-123).
fn draw_scrollbar(
    surface: &mut Surface,
    x: i32,
    y: i32,
    h: i32,
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
    crate::draw_image_strip(surface, x, y + 16, scroll, 16, 16, 16, 16, gamma);
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

        if state.current_page() != AboutPage::Credits {
            return;
        }

        // 7+. Credits sections: caption label, then the TextWindow rows
        // (no bg box / frame: SetDecoration(false,false,nullptr,true),
        // C4StartupAboutDlg.cpp:330-332).
        for (section, (caption, people)) in layout.sections.iter().zip(CREDITS_SECTIONS) {
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

            // TextWindow client = bounds inset T8/B8 (CustomMarginTextWindow
            // <0,8,0,8>, C4StartupAboutDlg.cpp:160-173); rows are TextFont,
            // white, ALeft, paragraph gap lh/3 before all but the first line
            // (MultilineLabel::DrawElement, C4GuiLabels.cpp:250-264), clipped
            // to the ScrollWindow bounds (Window::Draw, C4GuiContainers.cpp:
            // 273-296).
            let text_font = &fonts.text;
            let top = section.textbox.y + 8;
            let viewport_h = section.textbox.h - 16;
            let clip_y2 = top + viewport_h - 1;
            let mut y = top;
            for (index, person) in people.iter().enumerate() {
                if index > 0 {
                    y += text_font.line_height / 3;
                }
                if y > clip_y2 {
                    break;
                }
                text_font.draw_with_gamma(
                    surface,
                    section.textbox.x,
                    y,
                    &credit_line(*person),
                    WHITE,
                    TextAlign::Left,
                    true,
                    gamma,
                );
                y += text_font.line_height;
            }

            // Auto-hidden scrollbar: visible iff content overflows the
            // viewport (ScrollBar::Update, C4GuiContainers.cpp:343-368);
            // at (clientW-16, 0) within the TextWindow client.
            if content_height(text_font, people.len()) > viewport_h {
                draw_scrollbar(
                    surface,
                    section.textbox.x + section.textbox.w - SCROLL_BAR_WDT,
                    top,
                    viewport_h,
                    &assets.scroll,
                    gamma,
                );
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

    // Buttons invoke callbacks only after a matching down/up
    // (C4GuiButton.cpp:130-154). Update checking remains an external action
    // (`C4StartupAboutDlg::OnUpdateBtn`, cpp:377-380).
    #[test]
    fn live_state_hits_exact_about_buttons_and_emits_update_request() {
        let mut state = AboutDlgState::default();
        state.resize(1280, 720);
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
        state.resize(1280, 720);
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

    // No control has focus on first show. Therefore dialog Enter leaves;
    // Tab selects Back, Update, Licenses in add order, and a focused button
    // activates on key-up (C4GuiDialogs.cpp:616-646; C4GuiButton.cpp:112-128).
    #[test]
    fn live_state_keyboard_focus_and_activation_match_about_dialog() {
        let mut state = AboutDlgState::default();
        state.resize(1280, 720);
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
