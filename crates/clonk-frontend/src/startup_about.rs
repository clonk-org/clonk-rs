use crate::{draw_text, fill_rect, GuiPoint, KeyCode};
use clonk_graphics::{Color, Surface, TextFont};
use clonk_gui::{Rect as GuiRect, Size as GuiSize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AboutAction {
    Back,
    CheckForUpdates,
    NextPage,
}

pub struct StartupAboutDialog {
    font: Arc<dyn TextFont>,
    size: GuiSize,
    layout: Option<Layout>,
    pointer_position: Option<GuiPoint>,
    hovered_button: Option<ButtonKind>,
    pressed_button: Option<ButtonKind>,
    current_page: usize,
}

struct Layout {
    panel: GuiRect,
    credits_column1: Vec<CreditSection>,
    credits_column2: Vec<CreditSection>,
    credits_column3: Vec<CreditSection>,
    licenses_area: GuiRect,
    buttons: Vec<Button>,
}

#[derive(Clone, Copy, Debug)]
struct Button {
    rect: GuiRect,
    kind: ButtonKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonKind {
    Back,
    CheckForUpdates,
    Advance,
}

#[derive(Clone, Debug)]
struct CreditSection {
    title: &'static str,
    rect: GuiRect,
    entries: &'static [(&'static str, Option<&'static str>)],
}

impl ButtonKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Back => "Back",
            Self::CheckForUpdates => "Check for Updates",
            Self::Advance => "Licenses",
        }
    }
}

// Developer lists matching C++ exactly
const GAME_DESIGN: &[(&str, Option<&str>)] = &[("Matthes Bender", Some("matthes"))];

const CODE: &[(&str, Option<&str>)] = &[
    ("Sven Eberhardt", Some("Sven2")),
    ("Peter Wortmann", Some("PeterW")),
    ("Günther Brammer", Some("Günther")),
    ("Armin Burgmeier", Some("Clonk-Karl")),
    ("Julian Raschke", Some("survivor")),
    ("Alexander Post", Some("qualle")),
    ("Jan Heberer", Some("Jan")),
    ("Markus Mittendrein", Some("Der Tod")),
    ("Dominik Bayerl", Some("Kanibal")),
    ("George Tokmaji", Some("Fulgen")),
    ("Martin Plicht", Some("Mortimer")),
    ("Matthias Brehmer", Some("Bratkartoffl")),
    ("Tim Kuhrt", Some("TLK")),
];

const SCRIPTING: &[(&str, Option<&str>)] = &[
    ("Felix Wagner", Some("Clonkonaut")),
    ("Richard Gerum", Some("Randrian")),
    ("Markus Hoppe", Some("Shamino")),
    ("David Dormagen", Some("Zapper")),
    ("Florian Groß", Some("flgr")),
    ("Tobias Zwick", Some("Newton")),
    ("Bernhard Bonigl", Some("boni")),
    ("Viktor Yuschuk", Some("Viktor")),
    ("Raven", None),
];

const ADDITIONAL_ART: &[(&str, Option<&str>)] = &[
    ("Erik Nitzschke", Some("DukeAufDune")),
    ("Merten Ehmig", Some("pluto")),
    ("Matthias Rottländer", Some("Matthi")),
    ("Christopher Reimann", Some("Benzol")),
    ("Jonathan Veit", Some("AniProGuy")),
    ("Arthur Möller", Some("Aqua")),
    ("Tobias Zwick", Some("Newton")),
    ("Raven", None),
];

const MUSIC: &[(&str, Option<&str>)] = &[
    ("Hans-Christian Kühl", Some("HCK")),
    ("Sebastian Burkhart", Some("hypo")),
    ("Florian Boos", Some("Flobby")),
    ("Martin Strohmeier", Some("K-Pone")),
];

const VOICE: &[(&str, Option<&str>)] = &[("Klemens Köhring", None)];

const WEB: &[(&str, Option<&str>)] = &[
    ("Markus Wichitill", Some("mawic")),
    ("Martin Schuster", Some("knight_k")),
    ("Arne Bochem", Some("ArneB")),
    ("Lukas Werling", Some("Luchs")),
    ("Florian Graier", Some("Nachtfalter")),
    ("Benedict Etzel", Some("B_E")),
];

// Colors matching C++ StartupOptions
const PANEL_SHADOW_COLOR: Color = Color::new(0, 0, 0, 120);
const PANEL_BACKGROUND_COLOR: Color = Color::new(16, 28, 52, 235);
const TEXT_PRIMARY_COLOR: Color = Color::new(232, 238, 255, 255);
const TEXT_SECONDARY_COLOR: Color = Color::new(196, 206, 226, 255);
const TEXT_NICK_COLOR: Color = Color::new(247, 247, 111, 255);
const BUTTON_NORMAL_COLOR: Color = Color::new(36, 62, 104, 230);
const BUTTON_HOVER_COLOR: Color = Color::new(54, 90, 160, 240);
const BUTTON_PRESSED_COLOR: Color = Color::new(44, 70, 120, 240);
const FOOTER_BACKGROUND_COLOR: Color = Color::new(8, 14, 28, 210);
const LICENSE_BACKGROUND_COLOR: Color = Color::new(20, 32, 56, 230);

const TITLE_FONT_SIZE: f32 = 24.0;
const CAPTION_FONT_SIZE: f32 = 20.0;
const NAME_FONT_SIZE: f32 = 17.0;
const BUTTON_FONT_SIZE: f32 = 18.0;
const COPYRIGHT_FONT_SIZE: f32 = 13.0;
const LICENSE_FONT_SIZE: f32 = 15.0;

const BUTTON_HEIGHT: f32 = 38.0;
const BUTTON_SPACING: f32 = 12.0;
const PANEL_PADDING: f32 = 32.0;

const FANPROJECT_TEXT: &str = "Clonk Rust is a fan project based on Clonk Rage.";
const TRADEMARK_TEXT: &str = "'Clonk' is a registered trademark of Matthes Bender.";

const ISC_LICENSE: &str = "\
Copyright (c) 2001-2009, RedWolf Design GmbH, http://www.clonk.de/
Copyright (c) 2010-2016, The OpenClonk Team and contributors
Copyright (c) 2018-2025, The LegacyClonk Team and contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.";

impl StartupAboutDialog {
    pub fn new(font: Arc<dyn TextFont>) -> Self {
        Self {
            font,
            size: GuiSize::new(0.0, 0.0),
            layout: None,
            pointer_position: None,
            hovered_button: None,
            pressed_button: None,
            current_page: 0,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        let clamped_width = width.max(1.0);
        let clamped_height = height.max(1.0);
        if (self.size.width - clamped_width).abs() > f32::EPSILON
            || (self.size.height - clamped_height).abs() > f32::EPSILON
        {
            self.size = GuiSize::new(clamped_width, clamped_height);
            self.layout = None;
        }
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub fn current_page(&self) -> usize {
        self.current_page
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<AboutAction> {
        self.pointer_position = Some(position);
        self.update_hover(position);
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<AboutAction> {
        if let Some(button) = self.hit_test_button(position) {
            self.pressed_button = Some(button);
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<AboutAction> {
        let mut actions = Vec::new();
        let Some(pressed) = self.pressed_button.take() else {
            return actions;
        };
        if let Some(hit) = self.hit_test_button(position) {
            if hit == pressed {
                actions.extend(self.handle_button_click(hit));
            }
        }
        actions
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<AboutAction> {
        match key {
            KeyCode::Escape => {
                if self.current_page > 0 {
                    self.current_page = 0;
                    self.layout = None;
                    Vec::new()
                } else {
                    vec![AboutAction::Back]
                }
            }
            KeyCode::Enter | KeyCode::Space => {
                if self.current_page == 0 {
                    self.current_page = 1;
                    self.layout = None;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, _key: KeyCode) -> Vec<AboutAction> {
        Vec::new()
    }

    pub fn render(&mut self, surface: &mut Surface) {
        if self.layout.is_none() {
            self.layout = Some(self.compute_layout());
        }

        let Some(layout) = self.layout.as_ref() else {
            return;
        };

        // Draw shadow
        let shadow = GuiRect::new(
            layout.panel.origin.x + 4.0,
            layout.panel.origin.y + 4.0,
            layout.panel.size.width,
            layout.panel.size.height,
        );
        fill_rect(surface, &shadow, PANEL_SHADOW_COLOR);

        // Draw panel background
        fill_rect(surface, &layout.panel, PANEL_BACKGROUND_COLOR);

        // Draw title
        let title_rect = GuiRect::new(
            layout.panel.origin.x + PANEL_PADDING,
            layout.panel.origin.y + PANEL_PADDING,
            layout.panel.size.width - 2.0 * PANEL_PADDING,
            TITLE_FONT_SIZE * 1.5,
        );
        draw_text(
            surface,
            &title_rect,
            "About",
            TEXT_PRIMARY_COLOR,
            TITLE_FONT_SIZE,
            8.0,
            self.font.as_ref(),
        );

        match self.current_page {
            0 => self.render_credits_page(surface, layout),
            1 => self.render_licenses_page(surface, layout),
            _ => {}
        }

        // Draw buttons
        self.render_buttons(surface, layout);

        // Draw copyright footer
        self.render_footer(surface, layout);
    }

    fn render_credits_page(&self, surface: &mut Surface, layout: &Layout) {
        for section in &layout.credits_column1 {
            self.render_credit_section(surface, section);
        }
        for section in &layout.credits_column2 {
            self.render_credit_section(surface, section);
        }
        for section in &layout.credits_column3 {
            self.render_credit_section(surface, section);
        }
    }

    fn render_credit_section(&self, surface: &mut Surface, section: &CreditSection) {
        // Draw section title
        let title_rect = GuiRect::new(
            section.rect.origin.x,
            section.rect.origin.y,
            section.rect.size.width,
            CAPTION_FONT_SIZE * 1.4,
        );
        draw_text(
            surface,
            &title_rect,
            section.title,
            TEXT_SECONDARY_COLOR,
            CAPTION_FONT_SIZE,
            6.0,
            self.font.as_ref(),
        );

        // Draw names
        let mut y = section.rect.origin.y + CAPTION_FONT_SIZE * 1.8;
        let line_height = NAME_FONT_SIZE * 1.4;

        for (name, nick) in section.entries {
            let entry_rect = GuiRect::new(
                section.rect.origin.x + 8.0,
                y,
                section.rect.size.width - 8.0,
                line_height,
            );

            let text = match nick {
                Some(nick_str) => format!("{} ({})", name, nick_str),
                None => (*name).to_string(),
            };

            draw_text(
                surface,
                &entry_rect,
                &text,
                TEXT_PRIMARY_COLOR,
                NAME_FONT_SIZE,
                4.0,
                self.font.as_ref(),
            );

            y += line_height;
        }
    }

    fn render_licenses_page(&self, surface: &mut Surface, layout: &Layout) {
        // Draw license background
        fill_rect(surface, &layout.licenses_area, LICENSE_BACKGROUND_COLOR);

        // Draw license title
        let title_rect = GuiRect::new(
            layout.licenses_area.origin.x + 16.0,
            layout.licenses_area.origin.y + 16.0,
            layout.licenses_area.size.width - 32.0,
            CAPTION_FONT_SIZE * 1.5,
        );
        draw_text(
            surface,
            &title_rect,
            "ISC License",
            TEXT_SECONDARY_COLOR,
            CAPTION_FONT_SIZE,
            6.0,
            self.font.as_ref(),
        );

        // Draw license text
        let text_y = layout.licenses_area.origin.y + 56.0;
        let line_height = LICENSE_FONT_SIZE * 1.4;
        let mut y = text_y;

        for line in ISC_LICENSE.lines() {
            let line_rect = GuiRect::new(
                layout.licenses_area.origin.x + 16.0,
                y,
                layout.licenses_area.size.width - 32.0,
                line_height,
            );
            draw_text(
                surface,
                &line_rect,
                line,
                TEXT_PRIMARY_COLOR,
                LICENSE_FONT_SIZE,
                4.0,
                self.font.as_ref(),
            );
            y += line_height;
        }
    }

    fn render_buttons(&self, surface: &mut Surface, layout: &Layout) {
        for button in &layout.buttons {
            // Skip Advance button on page 1
            if button.kind == ButtonKind::Advance && self.current_page != 0 {
                continue;
            }

            let color = if self.pressed_button == Some(button.kind) {
                BUTTON_PRESSED_COLOR
            } else if self.hovered_button == Some(button.kind) {
                BUTTON_HOVER_COLOR
            } else {
                BUTTON_NORMAL_COLOR
            };

            fill_rect(surface, &button.rect, color);

            draw_text(
                surface,
                &button.rect,
                button.kind.label(),
                TEXT_PRIMARY_COLOR,
                BUTTON_FONT_SIZE,
                8.0,
                self.font.as_ref(),
            );
        }
    }

    fn render_footer(&self, surface: &mut Surface, layout: &Layout) {
        let footer_rect = GuiRect::new(
            layout.panel.origin.x + 16.0,
            layout.panel.origin.y + layout.panel.size.height - 36.0,
            layout.panel.size.width - 32.0,
            28.0,
        );
        fill_rect(surface, &footer_rect, FOOTER_BACKGROUND_COLOR);

        let footer_text = format!("{}   {}", FANPROJECT_TEXT, TRADEMARK_TEXT);
        draw_text(
            surface,
            &footer_rect,
            &footer_text,
            TEXT_SECONDARY_COLOR,
            COPYRIGHT_FONT_SIZE,
            8.0,
            self.font.as_ref(),
        );
    }

    fn compute_layout(&self) -> Layout {
        let width = self.size.width;
        let height = self.size.height;

        let panel_width = (width * 0.85).clamp(600.0, 1200.0);
        let panel_height = (height * 0.85).clamp(400.0, 900.0);
        let panel_x = (width - panel_width) / 2.0;
        let panel_y = (height - panel_height) / 2.0;

        let panel = GuiRect::new(panel_x, panel_y, panel_width, panel_height);

        // Button layout
        let button_y = panel_y + panel_height - 80.0;
        let button_width = 180.0;
        let total_button_width = button_width * 3.0 + BUTTON_SPACING * 2.0;
        let button_start_x = panel_x + (panel_width - total_button_width) / 2.0;

        let buttons = vec![
            Button {
                rect: GuiRect::new(button_start_x, button_y, button_width, BUTTON_HEIGHT),
                kind: ButtonKind::Back,
            },
            Button {
                rect: GuiRect::new(
                    button_start_x + button_width + BUTTON_SPACING,
                    button_y,
                    button_width,
                    BUTTON_HEIGHT,
                ),
                kind: ButtonKind::CheckForUpdates,
            },
            Button {
                rect: GuiRect::new(
                    button_start_x + (button_width + BUTTON_SPACING) * 2.0,
                    button_y,
                    button_width,
                    BUTTON_HEIGHT,
                ),
                kind: ButtonKind::Advance,
            },
        ];

        // Credits layout - 3 columns
        let content_y = panel_y + PANEL_PADDING + TITLE_FONT_SIZE * 2.0;
        let content_height = button_y - content_y - 60.0;
        let column_width = (panel_width - 2.0 * PANEL_PADDING - 40.0) / 3.0;

        let col1_x = panel_x + PANEL_PADDING;
        let col2_x = col1_x + column_width + 20.0;
        let col3_x = col2_x + column_width + 20.0;

        let credits_column1 = vec![
            CreditSection {
                title: "Game Design",
                rect: GuiRect::new(col1_x, content_y, column_width, content_height * 0.2),
                entries: GAME_DESIGN,
            },
            CreditSection {
                title: "Engine and Tools",
                rect: GuiRect::new(
                    col1_x,
                    content_y + content_height * 0.2,
                    column_width,
                    content_height * 0.8,
                ),
                entries: CODE,
            },
        ];

        let credits_column2 = vec![
            CreditSection {
                title: "Scripting",
                rect: GuiRect::new(col2_x, content_y, column_width, content_height * 0.5),
                entries: SCRIPTING,
            },
            CreditSection {
                title: "Additional Art",
                rect: GuiRect::new(
                    col2_x,
                    content_y + content_height * 0.5,
                    column_width,
                    content_height * 0.5,
                ),
                entries: ADDITIONAL_ART,
            },
        ];

        let credits_column3 = vec![
            CreditSection {
                title: "Music",
                rect: GuiRect::new(col3_x, content_y, column_width, content_height * 0.33),
                entries: MUSIC,
            },
            CreditSection {
                title: "Voice",
                rect: GuiRect::new(
                    col3_x,
                    content_y + content_height * 0.33,
                    column_width,
                    content_height * 0.3,
                ),
                entries: VOICE,
            },
            CreditSection {
                title: "Web",
                rect: GuiRect::new(
                    col3_x,
                    content_y + content_height * 0.63,
                    column_width,
                    content_height * 0.37,
                ),
                entries: WEB,
            },
        ];

        // Licenses layout
        let licenses_area = GuiRect::new(
            col1_x,
            content_y,
            panel_width - 2.0 * PANEL_PADDING,
            content_height,
        );

        Layout {
            panel,
            credits_column1,
            credits_column2,
            credits_column3,
            licenses_area,
            buttons,
        }
    }

    fn update_hover(&mut self, position: GuiPoint) {
        self.hovered_button = self.hit_test_button(position);
    }

    fn hit_test_button(&self, point: GuiPoint) -> Option<ButtonKind> {
        let Some(layout) = self.layout.as_ref() else {
            return None;
        };

        for button in &layout.buttons {
            // Skip Advance button on page 1
            if button.kind == ButtonKind::Advance && self.current_page != 0 {
                continue;
            }

            if point.x >= button.rect.origin.x
                && point.y >= button.rect.origin.y
                && point.x < button.rect.origin.x + button.rect.size.width
                && point.y < button.rect.origin.y + button.rect.size.height
            {
                return Some(button.kind);
            }
        }
        None
    }

    fn handle_button_click(&mut self, button: ButtonKind) -> Vec<AboutAction> {
        match button {
            ButtonKind::Back => {
                if self.current_page > 0 {
                    self.current_page = 0;
                    self.layout = None;
                    Vec::new()
                } else {
                    vec![AboutAction::Back]
                }
            }
            ButtonKind::CheckForUpdates => vec![AboutAction::CheckForUpdates],
            ButtonKind::Advance => {
                if self.current_page == 0 {
                    self.current_page = 1;
                    self.layout = None;
                }
                Vec::new()
            }
        }
    }
}
