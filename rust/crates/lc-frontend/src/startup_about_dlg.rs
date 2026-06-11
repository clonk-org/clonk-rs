//! Pixel-parity renderer for one C++ startup dialog (see
//! `rust/target/parity-specs/`). Implemented against the engine's F9
//! reference captures; owned by its implementation agent.
//!
//! Dialog: `C4StartupAboutDlg` (C4StartupAboutDlg.cpp). All geometry mirrors
//! the C++ integer math exactly; see `rust/target/parity-specs/about.md`.

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
use crate::startup_main_menu::{draw_bar, IntRect};
use crate::ImageData;
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

/// Renders the first-shown state of C4StartupAboutDlg (page 0, no fade, no
/// hover/focus) in the exact C++ draw order (about.md §"Exact draw order").
pub struct AboutDlgScreen;

impl AboutDlgScreen {
    pub fn render(
        surface: &mut Surface,
        assets: &AboutDlgAssets,
        fonts: &ClonkFontSet,
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
        for (rect, label) in layout.buttons.iter().zip(BUTTON_LABELS) {
            let bar = GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
            draw_bar(surface, &bar, &assets.button, gamma);
            let font = fonts.button_font(rect.h);
            let (caption, _) = expand_hotkey_markup(label);
            let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
            font.draw_with_gamma(
                surface,
                (rect.x + x1) / 2,
                (rect.y + y1 - font.line_height) / 2,
                &caption,
                YELLOW,
                TextAlign::Center,
                true,
                gamma,
            );
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
