//! Classic fullscreen network-lobby chrome from `C4GameLobby::MainDlg`.
//!
//! This is deliberately the base lobby slice: the fullscreen loader,
//! title, chat area, player-list sheet, bottom buttons and ready checkbox.
//! Resource/options/scenario sheets, lobby chat history and the full
//! `C4PlayerInfoListBox` row model remain separate follow-up surfaces.

use crate::classic_gui::{draw_3d_frame, draw_engine_box, ClassicButtonState, ClassicGuiSkin};
use crate::startup_main_menu::IntRect;
use crate::{ClonkFontSet, ImageData};
use anyhow::{ensure, Result};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{GammaRamp, Surface};
use lc_gui::Rect as GuiRect;

const BUTTON_HEIGHT: i32 = 32;
const ICON_EX_HEIGHT: i32 = 64;
const TEXT_LINE_HEIGHT: i32 = 22;
const TITLE_LINE_HEIGHT: i32 = 34;
const WOODEN_LABEL_HEIGHT: i32 = 23;
const EDIT_HEIGHT: i32 = 25;
const DARK_BACKGROUND: u32 = 0x7f00_0000;
const YELLOW: [u8; 4] = [0xff, 0xff, 0x00, 0xff];
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const LOG_GREY: [u8; 4] = [0xaf, 0xaf, 0xaf, 0xff];

/// Graphics.c4g resources used by the first visible lobby sheet.
pub struct GameLobbyAssets {
    /// The currently available loader image. C++ draws the scenario loader
    /// behind the fullscreen dialog (`C4Gui.cpp:669-682`).
    pub background: ImageData,
    pub caption: ImageData,
    pub button: ImageData,
    pub button_down: ImageData,
    pub button_highlight: ImageData,
    pub checkbox: ImageData,
}

/// One visible client row in the initial players sheet.
#[derive(Clone, Copy, Debug)]
pub struct GameLobbyParticipant<'a> {
    pub name: &'a str,
    pub ready: bool,
    pub local: bool,
}

/// Dynamic values drawn over the fixed MainDlg geometry.
pub struct GameLobbyRenderState<'a> {
    pub scenario_title: Option<&'a str>,
    pub participants: &'a [GameLobbyParticipant<'a>],
    pub local_ready: bool,
    pub is_host: bool,
    pub ready_highlighted: bool,
    pub start_highlighted: bool,
    pub start_pressed: bool,
    pub start_enabled: bool,
}

/// Exact first-sheet geometry in absolute screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameLobbyLayout {
    pub client: IntRect,
    pub title_anchor: (i32, i32),
    pub exit_button: IntRect,
    pub start_button: Option<IntRect>,
    pub ready_checkbox: IntRect,
    pub option_buttons: IntRect,
    pub players_caption: IntRect,
    pub players_list: IntRect,
    pub chat_log: IntRect,
    pub chat_caption: IntRect,
    pub chat_edit: IntRect,
}

#[derive(Clone, Copy)]
struct Aligner {
    area: IntRect,
    margin_x: i32,
    margin_y: i32,
}

impl Aligner {
    const fn new(area: IntRect, margin_x: i32, margin_y: i32) -> Self {
        Self {
            area,
            margin_x,
            margin_y,
        }
    }

    fn get_from_left(&mut self, width: i32) -> IntRect {
        let result = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - 2 * self.margin_y,
        };
        self.area.x += width + 2 * self.margin_x;
        self.area.w -= width + 2 * self.margin_x;
        result
    }

    fn get_from_right(&mut self, width: i32) -> IntRect {
        let result = IntRect {
            x: self.area.x + self.area.w - width - self.margin_x,
            y: self.area.y + self.margin_y,
            w: width,
            h: self.area.h - 2 * self.margin_y,
        };
        self.area.w -= width + 2 * self.margin_x;
        result
    }

    fn get_from_top(&mut self, height: i32) -> IntRect {
        let result = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: height,
        };
        self.area.y += height + 2 * self.margin_y;
        self.area.h -= height + 2 * self.margin_y;
        result
    }

    fn get_from_bottom(&mut self, height: i32) -> IntRect {
        let result = IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.area.h - height - self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: height,
        };
        self.area.h -= height + 2 * self.margin_y;
        result
    }

    const fn all(self) -> IntRect {
        IntRect {
            x: self.area.x + self.margin_x,
            y: self.area.y + self.margin_y,
            w: self.area.w - 2 * self.margin_x,
            h: self.area.h - 2 * self.margin_y,
        }
    }

    fn centered(self, width: i32, height: i32) -> IntRect {
        IntRect {
            x: self.area.x + self.area.w / 2 - width / 2,
            y: self.area.y + self.area.h / 2 - height / 2,
            w: width,
            h: height,
        }
    }

    fn expand_top(&mut self, height: i32) {
        self.area.y -= height;
        self.area.h += height;
    }
}

const fn offset(rect: IntRect, x: i32, y: i32) -> IntRect {
    IntRect {
        x: rect.x + x,
        y: rect.y + y,
        ..rect
    }
}

/// Mirrors `MainDlg::MainDlg`'s `ComponentAligner` calls
/// (`C4GameLobby.cpp:154-278`) and fullscreen margins
/// (`C4GuiDialogs.cpp:813-822,858-862`).
pub fn game_lobby_layout(width: i32, height: i32, is_host: bool) -> GameLobbyLayout {
    let margin_x = if width < 500 { 2 } else { width / 50 };
    let margin_y = if height < 320 { 2 } else { height * 2 / 75 };
    let margin_top = 50 + margin_y;
    let client = IntRect {
        x: margin_x,
        y: margin_top,
        w: width - 2 * margin_x,
        h: height - margin_top - margin_y,
    };
    let at_client = |rect| offset(rect, client.x, client.y);

    let (indent_x1, indent_x2, indent_x3, client_list_width) = if client.w > 500 {
        (10, 20, 5, client.w / 3)
    } else {
        (2, 2, 1, client.w / 2)
    };
    let (indent_y1, indent_y2, indent_y3, indent_y4) = if client.h > 320 {
        (16, 20, 8, 8)
    } else {
        (2, 2, 1, 1)
    };

    // fZeroAreaXY=true: all constructor component bounds are client-relative.
    let mut main = Aligner::new(
        IntRect {
            x: 0,
            y: 0,
            w: client.w,
            h: client.h,
        },
        0,
        0,
    );
    main.get_from_bottom(indent_y2);
    let bottom = main.get_from_bottom(BUTTON_HEIGHT + 2 * indent_y1);
    let mut bottom = Aligner::new(bottom, indent_x1, indent_y1);
    let exit_button = at_client(bottom.get_from_left(100));
    let start_button = is_host.then(|| at_client(bottom.get_from_right(100)));
    let ready_checkbox = at_client(bottom.get_from_right(110));
    if !is_host {
        bottom.get_from_left(10);
    }
    let option_buttons = at_client(bottom.centered(
        bottom.area.w - 2 * indent_x1,
        ICON_EX_HEIGHT.min(bottom.area.h),
    ));

    let right = main.get_from_right(client_list_width);
    let mut right = Aligner::new(right, indent_x3, indent_y4);
    let players_caption = at_client(right.get_from_top(WOODEN_LABEL_HEIGHT));
    right.expand_top(indent_y4 * 2 + 1);
    let players_list = at_client(right.all());

    let mut center = Aligner::new(main.all(), indent_x2, indent_y3);
    let chat_row = center.get_from_bottom(EDIT_HEIGHT);
    let mut chat_row = Aligner::new(chat_row, 0, 0);
    let chat_caption = at_client(chat_row.get_from_left(40));
    let chat_edit = at_client(chat_row.all());
    let chat_log = at_client(center.all());

    GameLobbyLayout {
        client,
        title_anchor: (client.x + client.w / 2, 50 / 2 - TITLE_LINE_HEIGHT / 2),
        exit_button,
        start_button,
        ready_checkbox,
        option_buttons,
        players_caption,
        players_list,
        chat_log,
        chat_caption,
        chat_edit,
    }
}

/// Draws the first visible `C4GameLobby::MainDlg` sheet.
pub fn render_game_lobby(
    surface: &mut Surface,
    assets: &GameLobbyAssets,
    fonts: &ClonkFontSet,
    state: &GameLobbyRenderState<'_>,
    gamma: Option<&GammaRamp>,
) -> Result<GameLobbyLayout> {
    ensure!(assets.background.width() > 0 && assets.background.height() > 0);
    ensure!(assets.caption.width() >= 64 && assets.caption.height() > 0);
    ensure!(assets.button.width() >= 64 && assets.button.height() > 0);
    ensure!(assets.button_down.width() >= 64 && assets.button_down.height() > 0);
    ensure!(assets.button_highlight.width() > 0 && assets.button_highlight.height() > 0);
    ensure!(
        assets.checkbox.height() > 0
            && assets.checkbox.width() >= assets.checkbox.height().saturating_mul(4)
    );

    let layout = game_lobby_layout(
        surface.width() as i32,
        surface.height() as i32,
        state.is_host,
    );
    crate::draw_image_bilinear(
        surface,
        &GuiRect::new(
            -1.0,
            -1.0,
            surface.width() as f32 + 2.0,
            surface.height() as f32 + 2.0,
        ),
        &assets.background,
        gamma,
    );

    let title = state
        .scenario_title
        .filter(|title| !title.is_empty())
        .map_or_else(|| "Lobby".to_string(), |title| format!("{title} - Lobby"));
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

    let skin = ClassicGuiSkin::new(
        &assets.caption,
        &assets.button,
        &assets.button_down,
        Some(&assets.button_highlight),
    );
    skin.draw_button(
        surface,
        layout.exit_button,
        "E&xit",
        fonts,
        ClassicButtonState::default(),
        gamma,
    );
    if let Some(start) = layout.start_button {
        skin.draw_button(
            surface,
            start,
            "&Start",
            fonts,
            ClassicButtonState {
                pressed: state.start_pressed && state.start_enabled,
                highlighted: state.start_highlighted && state.start_enabled,
            },
            gamma,
        );
    }

    draw_checkbox(
        surface,
        layout.ready_checkbox,
        "R&eady",
        state.local_ready,
        state.ready_highlighted,
        &assets.checkbox,
        &assets.button_highlight,
        fonts,
        gamma,
    );

    skin.draw_caption(
        surface,
        layout.players_caption,
        "Players",
        &fonts.text,
        YELLOW,
        TextAlign::Left,
        gamma,
    );
    draw_dark_frame(surface, layout.players_list, gamma);
    draw_participants(
        surface,
        layout.players_list,
        state.participants,
        fonts,
        gamma,
    );

    draw_dark_frame(surface, layout.chat_log, gamma);
    skin.draw_caption(
        surface,
        layout.chat_caption,
        "Cha&t:",
        &fonts.text,
        YELLOW,
        TextAlign::Center,
        gamma,
    );
    draw_dark_frame(surface, layout.chat_edit, gamma);

    Ok(layout)
}

fn draw_dark_frame(surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        rect.y + rect.h - 1,
        DARK_BACKGROUND,
        gamma,
    );
    draw_3d_frame(surface, rect, gamma);
}

#[allow(clippy::too_many_arguments)]
fn draw_checkbox(
    surface: &mut Surface,
    rect: IntRect,
    caption: &str,
    checked: bool,
    highlighted: bool,
    checkbox: &ImageData,
    highlight: &ImageData,
    fonts: &ClonkFontSet,
    gamma: Option<&GammaRamp>,
) {
    let cell = checkbox.height();
    let phase = u32::from(checked);
    crate::classic_gui::draw_facet_stretch(
        surface,
        checkbox,
        (phase as f32 * cell as f32, 0.0, cell as f32, cell as f32),
        (rect.x as f32, rect.y as f32, rect.h as f32, rect.h as f32),
        gamma,
    );
    let (text, _) = crate::expand_hotkey_markup(caption);
    fonts.text.draw_with_gamma(
        surface,
        rect.x + rect.h + 4,
        rect.y + (rect.h - fonts.text.line_height).max(0) / 2,
        &text,
        WHITE,
        TextAlign::Left,
        true,
        gamma,
    );
    if highlighted {
        let size = rect.h / 2;
        crate::draw_image_bilinear_additive(
            surface,
            &GuiRect::new(
                (rect.x + rect.h / 4) as f32,
                (rect.y + rect.h / 4) as f32,
                size as f32,
                size as f32,
            ),
            highlight,
            gamma,
        );
    }
}

fn draw_participants(
    surface: &mut Surface,
    list: IntRect,
    participants: &[GameLobbyParticipant<'_>],
    fonts: &ClonkFontSet,
    gamma: Option<&GammaRamp>,
) {
    let mut y = list.y + 4;
    for participant in participants {
        if y + TEXT_LINE_HEIGHT > list.y + list.h - 3 {
            break;
        }
        let color = if participant.local { YELLOW } else { WHITE };
        fonts.text.draw_with_gamma(
            surface,
            list.x + 5,
            y,
            participant.name,
            color,
            TextAlign::Left,
            true,
            gamma,
        );
        let status = if participant.ready {
            "Ready"
        } else {
            "Waiting"
        };
        fonts.mini.draw_with_gamma(
            surface,
            list.x + list.w - 6,
            y + (TEXT_LINE_HEIGHT - fonts.mini.line_height) / 2,
            status,
            LOG_GREY,
            TextAlign::Right,
            true,
            gamma,
        );
        y += TEXT_LINE_HEIGHT + 3;
    }
}
