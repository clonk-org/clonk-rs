//! Semantic layout producers for every presentation capture whose manifest
//! comparison term is `layout`.

use crate::presentation_layout::{
    LayoutElement, LayoutLine, LayoutRect, LayoutTrace, LAYOUT_TRACE_SCHEMA,
};
use clonk_frontend::{main_menu_layout, StartupMainMenu};
use clonk_graphics::clonk_font::{CapturedClonkText, ClonkFont, TextAlign};
use std::collections::BTreeSet;

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 720;
const RESOLUTION: &str = "1280x720";
const SCALE: u32 = 100;
const BRANDING: &str = "branding";
const SUPER_RESOLVED_STARTUP_ART: &str = "super-resolved-startup-art";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartupLayoutTraceError {
    MissingText {
        path: &'static str,
    },
    RuntimeMissingText {
        screen: &'static str,
        path: String,
    },
    RuntimeCaptionMismatch {
        screen: &'static str,
        path: String,
        expected: String,
        actual: String,
    },
    UnsupportedState {
        screen: &'static str,
        detail: &'static str,
    },
    DuplicatePath {
        path: String,
    },
    Serialization(String),
}

impl std::fmt::Display for StartupLayoutTraceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingText { path } => write!(formatter, "no rendered text for `{path}`"),
            Self::RuntimeMissingText { screen, path } => {
                write!(formatter, "no rendered text for `{path}` in `{screen}`")
            }
            Self::RuntimeCaptionMismatch {
                screen,
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "rendered text for `{path}` in `{screen}` is {actual:?}, expected {expected:?}"
            ),
            Self::UnsupportedState { screen, detail } => {
                write!(formatter, "cannot trace `{screen}`: {detail}")
            }
            Self::DuplicatePath { path } => write!(formatter, "duplicate layout path `{path}`"),
            Self::Serialization(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for StartupLayoutTraceError {}

fn layout_rect(x: i32, y: i32, width: i32, height: i32) -> LayoutRect {
    LayoutRect {
        x,
        y,
        width: width.max(0) as u32,
        height: height.max(0) as u32,
    }
}

fn trace(
    screen: &'static str,
    elements: Vec<LayoutElement>,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    let mut paths = BTreeSet::new();
    for element in &elements {
        if !paths.insert(element.path.clone()) {
            return Err(StartupLayoutTraceError::DuplicatePath {
                path: element.path.clone(),
            });
        }
    }
    Ok(LayoutTrace {
        schema: LAYOUT_TRACE_SCHEMA.to_string(),
        screen: screen.to_string(),
        resolution: RESOLUTION.to_string(),
        scale: SCALE,
        elements,
    })
}

fn empty_element(
    path: impl Into<String>,
    role: &'static str,
    rect: LayoutRect,
    port_asset: Option<&'static str>,
) -> LayoutElement {
    LayoutElement {
        path: path.into(),
        role: role.to_string(),
        rect,
        visible: true,
        port_asset: port_asset.map(str::to_string),
        caption: String::new(),
        lines: Vec::new(),
    }
}

fn split_command_lines(command: &CapturedClonkText) -> impl Iterator<Item = &str> {
    command
        .text
        .split(move |character| character == '\n' || (command.markup && character == '|'))
}

fn command_lines(command: &CapturedClonkText, font: &ClonkFont) -> Vec<LayoutLine> {
    let line_height = (command.zoom * font.line_height as f32) as i32;
    split_command_lines(command)
        .enumerate()
        .filter(|(_, text)| !text.is_empty())
        .map(|(index, text)| {
            let width = (command.zoom * font.measure(text, command.markup).0 as f32) as i32;
            let x = match command.align {
                TextAlign::Left => command.x,
                TextAlign::Center => command.x - width / 2,
                TextAlign::Right => command.x - width,
            };
            LayoutLine {
                text: text.to_string(),
                rect: layout_rect(
                    x,
                    command.y + i32::try_from(index).unwrap_or(i32::MAX) * line_height,
                    width,
                    line_height,
                ),
            }
        })
        .collect()
}

fn command_anchor_in(command: &CapturedClonkText, rect: LayoutRect) -> bool {
    let right = rect.x.saturating_add(rect.width as i32);
    let bottom = rect.y.saturating_add(rect.height as i32);
    let horizontal = match command.align {
        TextAlign::Right => command.x > rect.x && command.x <= right,
        TextAlign::Left | TextAlign::Center => command.x >= rect.x && command.x < right,
    };
    horizontal && command.y >= rect.y && command.y < bottom
}

fn text_element<'a>(
    path: &'static str,
    role: &'static str,
    rect: LayoutRect,
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let lines = commands
        .iter()
        .filter(|command| command_anchor_in(command, rect))
        .flat_map(|command| {
            font(command)
                .into_iter()
                .flat_map(|font| command_lines(command, font))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return Err(StartupLayoutTraceError::MissingText { path });
    }
    let caption = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(LayoutElement {
        path: path.to_string(),
        role: role.to_string(),
        rect,
        visible: true,
        port_asset: port_asset.map(str::to_string),
        caption,
        lines,
    })
}

fn union_line_rect(lines: &[LayoutLine]) -> LayoutRect {
    let left = lines
        .iter()
        .map(|line| line.rect.x)
        .min()
        .unwrap_or_default();
    let top = lines
        .iter()
        .map(|line| line.rect.y)
        .min()
        .unwrap_or_default();
    let right = lines
        .iter()
        .map(|line| line.rect.x.saturating_add(line.rect.width as i32))
        .max()
        .unwrap_or(left);
    let bottom = lines
        .iter()
        .map(|line| line.rect.y.saturating_add(line.rect.height as i32))
        .max()
        .unwrap_or(top);
    layout_rect(
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
}

fn anchored_text_element<'a>(
    path: &'static str,
    role: &'static str,
    anchor: (i32, i32),
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let command = commands
        .iter()
        .find(|command| command.x == anchor.0 && command.y == anchor.1)
        .ok_or(StartupLayoutTraceError::MissingText { path })?;
    let lines = font(command)
        .map(|font| command_lines(command, font))
        .filter(|lines| !lines.is_empty())
        .ok_or(StartupLayoutTraceError::MissingText { path })?;
    let caption = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(LayoutElement {
        path: path.to_string(),
        role: role.to_string(),
        rect: union_line_rect(&lines),
        visible: true,
        port_asset: port_asset.map(str::to_string),
        caption,
        lines,
    })
}

fn text_element_at_anchor<'a>(
    path: &'static str,
    role: &'static str,
    rect: LayoutRect,
    anchor: (i32, i32),
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let command = commands
        .iter()
        .find(|command| command.x == anchor.0 && command.y == anchor.1)
        .ok_or(StartupLayoutTraceError::MissingText { path })?;
    let lines = font(command)
        .map(|font| command_lines(command, font))
        .filter(|lines| !lines.is_empty())
        .ok_or(StartupLayoutTraceError::MissingText { path })?;
    let caption = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(LayoutElement {
        path: path.to_string(),
        role: role.to_string(),
        rect,
        visible: true,
        port_asset: port_asset.map(str::to_string),
        caption,
        lines,
    })
}

fn allocated_text_element_at_anchor<'a>(
    path: &'static str,
    role: &'static str,
    rect: LayoutRect,
    anchor: (i32, i32),
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let mut element = text_element_at_anchor(path, role, rect, anchor, port_asset, commands, font)?;
    for line in &mut element.lines {
        line.rect = rect;
    }
    Ok(element)
}

fn optional_text_element<'a>(
    path: impl Into<String>,
    role: &'static str,
    rect: LayoutRect,
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> LayoutElement {
    let lines = commands
        .iter()
        .filter(|command| command_anchor_in(command, rect))
        .flat_map(|command| {
            font(command)
                .into_iter()
                .flat_map(|font| command_lines(command, font))
        })
        .collect::<Vec<_>>();
    let caption = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    LayoutElement {
        path: path.into(),
        role: role.to_string(),
        rect,
        visible: true,
        port_asset: port_asset.map(str::to_string),
        caption,
        lines,
    }
}

fn label_at_anchor<'a>(
    path: &'static str,
    anchor: (i32, i32),
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    anchored_text_element(path, "label", anchor, port_asset, commands, font)
}

fn text_element_with_caption<'a>(
    path: &'static str,
    role: &'static str,
    caption: &str,
    port_asset: Option<&'static str>,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let command = commands
        .iter()
        .find(|command| command.text == caption)
        .ok_or(StartupLayoutTraceError::MissingText { path })?;
    anchored_text_element(
        path,
        role,
        (command.x, command.y),
        port_asset,
        commands,
        font,
    )
}

fn gui_font<'a>(
    fonts: &'a clonk_frontend::ClonkFontSet,
    command: &CapturedClonkText,
) -> Option<&'a ClonkFont> {
    use clonk_graphics::clonk_font::ClonkFontRole;
    match command.role {
        ClonkFontRole::GuiTitle => Some(&fonts.title),
        ClonkFontRole::GuiCaption => Some(&fonts.caption),
        ClonkFontRole::GuiText => Some(&fonts.text),
        ClonkFontRole::GuiMainSmall => Some(&fonts.main_small),
        ClonkFontRole::GuiMini => Some(&fonts.mini),
        _ => None,
    }
}

fn surface_layout_rect(rect: clonk_graphics::Rect) -> LayoutRect {
    LayoutRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn classic_layout_rect(rect: clonk_frontend::classic_gui::IntRect) -> LayoutRect {
    layout_rect(rect.x, rect.y, rect.w, rect.h)
}

/// Projects the HUD nodes selected by the completed renderer pass.
///
/// Pinned C++ oracle: `src/C4Viewport.cpp:891-961,1281-1533`.
fn runtime_hud_elements(
    snapshot: &clonk_frontend::RuntimePresentationSnapshot,
) -> Vec<LayoutElement> {
    use clonk_frontend::RuntimeHudPresentationNodeKind as Kind;

    snapshot
        .active_viewports
        .iter()
        .filter_map(|viewport| snapshot.hud_layout_for_viewport(viewport.index))
        .flat_map(|hud| {
            let base = format!("game/viewport/{}/hud", hud.viewport_index);
            let mouse_viewport = snapshot
                .viewport_control_overlays
                .is_some_and(|controls| controls.mouse_viewport_index == Some(hud.viewport_index));
            hud.nodes.into_iter().map(move |node| {
                let (path, role) = match node.kind {
                    Kind::CursorInfo => (format!("{base}/cursor-info"), "group"),
                    Kind::Portrait => (format!("{base}/cursor-info/portrait"), "image"),
                    Kind::Inventory => (format!("{base}/inventory"), "inventory"),
                    Kind::EnergyBar => (format!("{base}/energy"), "meter"),
                    Kind::MagicBar => (format!("{base}/magic-energy"), "meter"),
                    Kind::BreathBar => (format!("{base}/breath"), "meter"),
                    Kind::PrimaryCommands => (format!("{base}/commands/primary"), "command-area"),
                    Kind::SecondaryCommands => {
                        (format!("{base}/commands/secondary"), "command-area")
                    }
                    Kind::Wealth => (format!("{base}/player/wealth"), "value"),
                    Kind::Value => (format!("{base}/player/value"), "value"),
                    Kind::Crew => (format!("{base}/player/crew"), "value"),
                    Kind::ShowControl(control) => (format!("{base}/controls/{control}"), "control"),
                    Kind::ViewportHelp => (format!("{base}/mouse/help"), "control"),
                    Kind::ViewportPlayerMenu if mouse_viewport => {
                        (format!("{base}/mouse/player-menu"), "control")
                    }
                    Kind::ViewportPlayerMenu => (format!("{base}/player-menu"), "control"),
                };
                empty_element(path, role, surface_layout_rect(node.rect), None)
            })
        })
        .collect()
}

/// Produces the fullscreen runtime topology shared by HUD, gameplay, menu and
/// evaluation captures. Case-specific nodes are inserted after the viewport
/// landscape and before the global message board.
///
/// Pinned C++ oracle: `src/C4GraphicsSystem.cpp:352-365` and
/// `src/C4UpperBoard.cpp:125-180`.
pub(crate) fn runtime_base_trace(
    screen: &'static str,
    snapshot: &clonk_frontend::RuntimePresentationSnapshot,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
    case_elements: Vec<LayoutElement>,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    if snapshot.surface_rect != clonk_graphics::Rect::new(0, 0, WIDTH as u32, HEIGHT as u32) {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen,
            detail: "runtime surface is not the canonical capture geometry",
        });
    }
    let upper_board =
        snapshot
            .upper_board_output_rect
            .ok_or(StartupLayoutTraceError::UnsupportedState {
                screen,
                detail: "runtime upper board is hidden",
            })?;
    let message_board =
        snapshot
            .message_board_output_rect
            .ok_or(StartupLayoutTraceError::UnsupportedState {
                screen,
                detail: "runtime message board is hidden",
            })?;
    let logo_slot =
        snapshot
            .upper_board_logo_slot
            .ok_or(StartupLayoutTraceError::UnsupportedState {
                screen,
                detail: "runtime upper-board product logo is unavailable",
            })?;
    let [viewport] = snapshot.active_viewports.as_slice() else {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen,
            detail: "runtime capture does not have exactly one viewport",
        });
    };
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    let mut game_time = text_element_with_caption(
        "game/upper-board/game-time",
        "label",
        &snapshot.formatted_game_time,
        None,
        commands,
        &font,
    )?;
    let game_time_rect = layout_rect(
        upper_board.x + upper_board.width as i32 - snapshot.upper_board_text_width - 10,
        game_time.rect.y,
        snapshot.upper_board_text_width,
        game_time.rect.height as i32,
    );
    game_time.rect = game_time_rect;
    for line in &mut game_time.lines {
        line.rect = game_time_rect;
    }
    let mut elements = vec![
        empty_element(
            "game/surface",
            "surface",
            surface_layout_rect(snapshot.surface_rect),
            None,
        ),
        empty_element(
            "game/upper-board/background",
            "board",
            surface_layout_rect(upper_board),
            None,
        ),
        // The renderer aspect-fits the active Clonk Rust logo inside this
        // compatibility slot. The artwork is intentional branding and is
        // never substituted with the legacy asset.
        empty_element(
            "game/upper-board/branding/logo",
            "image",
            surface_layout_rect(logo_slot),
            Some(BRANDING),
        ),
        text_element_with_caption(
            "game/upper-board/scenario-title",
            "label",
            &snapshot.scenario_title,
            None,
            commands,
            &font,
        )?,
        game_time,
        empty_element(
            "game/viewport/0",
            "viewport",
            surface_layout_rect(viewport.rect),
            None,
        ),
        empty_element(
            "game/viewport/0/landscape",
            "landscape",
            surface_layout_rect(viewport.content_rect),
            None,
        ),
    ];
    elements.extend(runtime_hud_elements(snapshot));
    elements.extend(case_elements);
    elements.push(empty_element(
        "game/message-board/output",
        "board",
        surface_layout_rect(message_board),
        None,
    ));
    trace(screen, elements)
}

fn required_runtime_text_element<'a>(
    screen: &'static str,
    path: impl Into<String>,
    role: &'static str,
    rect: clonk_graphics::Rect,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let path = path.into();
    let element = optional_text_element(
        path.clone(),
        role,
        surface_layout_rect(rect),
        None,
        commands,
        font,
    );
    if element.lines.is_empty() {
        return Err(StartupLayoutTraceError::RuntimeMissingText { screen, path });
    }
    Ok(element)
}

fn required_runtime_caption_element<'a>(
    screen: &'static str,
    path: impl Into<String>,
    role: &'static str,
    rect: clonk_graphics::Rect,
    caption: &str,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let path = path.into();
    let command = commands
        .iter()
        .find(|command| command.text == caption)
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen,
            path: path.clone(),
        })?;
    let lines = font(command)
        .map(|font| command_lines(command, font))
        .filter(|lines| !lines.is_empty())
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen,
            path: path.clone(),
        })?;
    Ok(LayoutElement {
        path,
        role: role.to_string(),
        rect: surface_layout_rect(rect),
        visible: true,
        port_asset: None,
        caption: caption.to_string(),
        lines,
    })
}

fn required_runtime_caption_with_line_rect<'a>(
    screen: &'static str,
    path: impl Into<String>,
    role: &'static str,
    rect: clonk_graphics::Rect,
    line_rect: clonk_graphics::Rect,
    caption: &str,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let mut element =
        required_runtime_caption_element(screen, path, role, rect, caption, commands, font)?;
    for line in &mut element.lines {
        line.rect = surface_layout_rect(line_rect);
    }
    Ok(element)
}

fn hidden_native_menu_scrollbar(client: clonk_graphics::Rect) -> clonk_graphics::Rect {
    // C4GUI::ScrollWindow retains its constructed 100x100 client and 16px
    // scrollbar geometry while the bar is hidden. Menu InitSize does not
    // resize that latent child until overflow makes it visible.
    clonk_graphics::Rect::new(client.x + 84, client.y, 16, 100)
}

fn required_runtime_anchor_element<'a>(
    screen: &'static str,
    path: impl Into<String>,
    role: &'static str,
    rect: LayoutRect,
    anchor: (i32, i32),
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Result<LayoutElement, StartupLayoutTraceError> {
    let path = path.into();
    let command = commands
        .iter()
        .find(|command| (command.x, command.y) == anchor)
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen,
            path: path.clone(),
        })?;
    let lines = font(command)
        .map(|font| command_lines(command, font))
        .filter(|lines| !lines.is_empty())
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen,
            path: path.clone(),
        })?;
    Ok(LayoutElement {
        path,
        role: role.to_string(),
        rect,
        visible: true,
        port_asset: None,
        caption: command.text.clone(),
        lines,
    })
}

/// Projects one visible `C4MainMenu` from the exact `MenuLayout` consumed by
/// its completed runtime render.
///
/// Pinned C++ oracle: `src/C4Menu.cpp:642-783,796-880`.
pub(crate) fn runtime_ingame_menu_elements(
    menu: &clonk_app_menus::ingame_menu::IngameMenuState,
    layout: &clonk_app_menus::ingame_menu::IngameMenuPresentationLayout,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<Vec<LayoutElement>, StartupLayoutTraceError> {
    use clonk_app_menus::ingame_menu::IngameMenuControlKind;

    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    let mut elements = vec![
        empty_element(
            "game/viewport/0/menu/background",
            "panel",
            surface_layout_rect(layout.bounds),
            None,
        ),
        required_runtime_caption_element(
            "ingame-menu",
            "game/viewport/0/menu/title",
            "label",
            layout.title,
            menu.caption(),
            commands,
            &font,
        )?,
    ];
    if let Some(close_button) = layout.close_button {
        elements.push(empty_element(
            "game/viewport/0/menu/close",
            "button",
            surface_layout_rect(close_button),
            None,
        ));
    }
    elements.push(empty_element(
        "game/viewport/0/menu/client",
        "panel",
        surface_layout_rect(layout.client),
        None,
    ));
    for item in &layout.visible_items {
        let menu_item =
            menu.items()
                .get(item.index)
                .ok_or(StartupLayoutTraceError::UnsupportedState {
                    screen: "ingame-menu",
                    detail: "visible menu geometry references a missing item",
                })?;
        let line_rect = clonk_graphics::Rect::new(
            item.rect.x + item.rect.height as i32,
            item.rect.y,
            item.rect.width.saturating_sub(item.rect.height),
            item.rect.height,
        );
        let element = required_runtime_caption_with_line_rect(
            "ingame-menu",
            format!("game/viewport/0/menu/items/{}", item.index),
            "menu-item",
            item.rect,
            line_rect,
            &menu_item.caption,
            commands,
            &font,
        )?;
        elements.push(element);
    }
    let (scrollbar_rect, scrollbar_visible) = layout.scrollbar.map_or_else(
        || (hidden_native_menu_scrollbar(layout.client), false),
        |scrollbar| (scrollbar.rect, scrollbar.visible),
    );
    elements.push(LayoutElement {
        visible: scrollbar_visible,
        ..empty_element(
            "game/viewport/0/menu/scrollbar",
            "scrollbar",
            surface_layout_rect(scrollbar_rect),
            None,
        )
    });
    if let Some(extra_bar) = layout.extra_bar {
        elements.push(empty_element(
            "game/viewport/0/menu/extra-bar",
            "panel",
            surface_layout_rect(extra_bar),
            None,
        ));
    }
    for control in &layout.controls {
        let path = match control.kind {
            IngameMenuControlKind::EnterKey => "game/viewport/0/menu/controls/enter-key",
            IngameMenuControlKind::Confirm => "game/viewport/0/menu/controls/confirm",
            IngameMenuControlKind::CloseKey => "game/viewport/0/menu/controls/close-key",
            IngameMenuControlKind::Cancel => "game/viewport/0/menu/controls/cancel",
        };
        elements.push(optional_text_element(
            path,
            "control",
            surface_layout_rect(control.rect),
            None,
            commands,
            &font,
        ));
    }
    Ok(elements)
}

/// Projects one visible cursor-owned C4Script menu from the same initialized
/// geometry used by its completed runtime render.
///
/// Pinned C++ oracle: `src/C4Menu.cpp:642-880`.
pub(crate) fn runtime_object_menu_elements(
    menu: &clonk_engine::ObjectMenuState,
    layout: clonk_app_menus::object_menu::EngineScriptMenuLayout,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<Vec<LayoutElement>, StartupLayoutTraceError> {
    use clonk_app_menus::object_menu::EngineScriptMenuControlKind;

    const SCREEN: &str = "object-menu";
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    let mut elements = vec![
        empty_element(
            "game/viewport/0/menu/background",
            "panel",
            surface_layout_rect(layout.bounds),
            None,
        ),
        required_runtime_caption_element(
            SCREEN,
            "game/viewport/0/menu/title",
            "label",
            layout.title,
            &crate::c4_presentation_text(&menu.caption),
            commands,
            &font,
        )?,
        empty_element(
            "game/viewport/0/menu/close",
            "button",
            surface_layout_rect(layout.close_button_rect()),
            None,
        ),
        empty_element(
            "game/viewport/0/menu/client",
            "panel",
            surface_layout_rect(layout.client),
            None,
        ),
    ];
    for (index, item) in menu.items.iter().enumerate() {
        let Some(rect) = layout.item_rect(index) else {
            continue;
        };
        let line_rect = clonk_graphics::Rect::new(
            rect.x + layout.item_height,
            rect.y,
            rect.width.saturating_sub(layout.item_height.max(0) as u32),
            rect.height,
        );
        elements.push(required_runtime_caption_with_line_rect(
            SCREEN,
            format!("game/viewport/0/menu/items/{index}"),
            "menu-item",
            rect,
            line_rect,
            &crate::c4_presentation_text(&item.caption),
            commands,
            &font,
        )?);
    }
    let scrollbar = layout
        .scrollbar
        .unwrap_or_else(|| hidden_native_menu_scrollbar(layout.client));
    elements.push(LayoutElement {
        visible: layout.scrollbar.is_some(),
        ..empty_element(
            "game/viewport/0/menu/scrollbar",
            "scrollbar",
            surface_layout_rect(scrollbar),
            None,
        )
    });
    if let Some(strip) = layout.command_strip_layout(menu) {
        elements.push(empty_element(
            "game/viewport/0/menu/extra-bar",
            "panel",
            surface_layout_rect(strip.rect),
            None,
        ));
        for control in strip.controls {
            let name = match control.kind {
                EngineScriptMenuControlKind::EnterKey => "enter-key",
                EngineScriptMenuControlKind::Confirm => "confirm",
                EngineScriptMenuControlKind::EnterAllKey => "enter-all-key",
                EngineScriptMenuControlKind::ConfirmAll => "confirm-all",
                EngineScriptMenuControlKind::CloseKey => "close-key",
                EngineScriptMenuControlKind::Cancel => "cancel",
                EngineScriptMenuControlKind::Exit => "exit",
            };
            elements.push(empty_element(
                format!("game/viewport/0/menu/controls/{name}"),
                "control",
                surface_layout_rect(control.rect),
                None,
            ));
        }
    }
    Ok(elements)
}

pub(crate) fn runtime_global_message_elements(
    index: usize,
    layout: &crate::game_message::GlobalMessagePresentationLayout,
) -> Vec<LayoutElement> {
    let base = format!("game/viewport/0/message/{index}");
    vec![
        empty_element(
            format!("{base}/background"),
            "panel",
            surface_layout_rect(layout.background),
            None,
        ),
        empty_element(
            format!("{base}/portrait"),
            "image",
            surface_layout_rect(layout.portrait),
            None,
        ),
        LayoutElement {
            path: format!("{base}/text"),
            role: "label".to_string(),
            rect: surface_layout_rect(layout.text),
            visible: true,
            port_asset: None,
            caption: layout.caption.clone(),
            lines: vec![LayoutLine {
                text: layout.caption.clone(),
                rect: surface_layout_rect(layout.text),
            }],
        },
    ]
}

/// Projects the classic game-over dialog and its first live player list from
/// the exact layout consumed by `GameOverState::render_classic`.
///
/// Pinned C++ oracle: `src/C4GameOverDlg.cpp:115-258` and
/// `src/C4PlayerInfoListBox.cpp:79-154,184-231`.
pub(crate) fn runtime_evaluation_elements(
    state: &clonk_app_menus::game_over::GameOverState,
    layout: &clonk_app_menus::game_over::ClassicGameOverPresentationLayout,
    settlement_score_icon: Option<&clonk_frontend::ImageData>,
    league_score_icon: Option<&clonk_frontend::ImageData>,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<Vec<LayoutElement>, StartupLayoutTraceError> {
    const SCREEN: &str = "evaluation";
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    let mut elements = vec![
        empty_element(
            "game/evaluation/dialog",
            "dialog",
            classic_layout_rect(layout.dialog),
            None,
        ),
        required_runtime_caption_element(
            SCREEN,
            "game/evaluation/title",
            "label",
            clonk_graphics::Rect::new(
                layout.caption.x,
                layout.caption.y,
                layout.caption.w.max(0) as u32,
                layout.caption.h.max(0) as u32,
            ),
            "Evaluation",
            commands,
            &font,
        )?,
        empty_element(
            "game/evaluation/close",
            "button",
            classic_layout_rect(layout.close_button),
            None,
        ),
    ];

    if let Some(goal_area) = layout.goal_area {
        elements.push(empty_element(
            "game/evaluation/goals",
            "group",
            classic_layout_rect(goal_area),
            None,
        ));
        if let Some(goal) = layout.evaluation.goals.first() {
            elements.push(empty_element(
                "game/evaluation/goals/0/picture",
                "image",
                classic_layout_rect(goal.picture),
                None,
            ));
        }
    }

    let player_list = layout.evaluation.player_list_windows.first().ok_or(
        StartupLayoutTraceError::UnsupportedState {
            screen: SCREEN,
            detail: "evaluation has no player list",
        },
    )?;
    elements.extend([
        empty_element(
            "game/evaluation/player-list",
            "list",
            classic_layout_rect(player_list.area),
            None,
        ),
        empty_element(
            "game/evaluation/player-list/viewport",
            "viewport",
            classic_layout_rect(player_list.viewport),
            None,
        ),
    ]);

    let player =
        layout
            .evaluation
            .players
            .first()
            .ok_or(StartupLayoutTraceError::UnsupportedState {
                screen: SCREEN,
                detail: "evaluation has no player row",
            })?;
    let mut player_name = required_runtime_anchor_element(
        SCREEN,
        "game/evaluation/player-list/players/0/name",
        "label",
        layout_rect(
            player.name_anchor.0,
            player.name_anchor.1,
            player.row.x + player.row.w - player.name_anchor.0,
            gui_fonts.text.line_height,
        ),
        player.name_anchor,
        commands,
        &font,
    )?;
    player_name.rect = union_line_rect(&player_name.lines);
    elements.extend([
        empty_element(
            "game/evaluation/player-list/players/0",
            "list-item",
            classic_layout_rect(player.row),
            None,
        ),
        empty_element(
            "game/evaluation/player-list/players/0/portrait",
            "image",
            classic_layout_rect(player.icon),
            None,
        ),
        player_name,
        required_runtime_anchor_element(
            SCREEN,
            "game/evaluation/player-list/players/0/time",
            "label",
            layout_rect(
                player.icon.x + player.icon.w + 6,
                player.time_anchor.1,
                player.time_anchor.0 - (player.icon.x + player.icon.w + 6),
                gui_fonts.text.line_height + 4,
            ),
            player.time_anchor,
            commands,
            &font,
        )?,
    ]);
    let evaluation_player = state
        .evaluation()
        .players()
        .nth(player.player_index)
        .ok_or(StartupLayoutTraceError::UnsupportedState {
            screen: SCREEN,
            detail: "evaluation player row has no backing player",
        })?;
    let (score_icon_kind, score_text) =
        clonk_app_menus::game_over::evaluation_score_label(evaluation_player, "Score").ok_or(
            StartupLayoutTraceError::UnsupportedState {
                screen: SCREEN,
                detail: "evaluation player has no visible score",
            },
        )?;
    let (score_icon_name, score_icon) = match score_icon_kind {
        clonk_app_menus::game_over::EvaluationScoreIcon::League => ("League", league_score_icon),
        clonk_app_menus::game_over::EvaluationScoreIcon::Settlement => {
            ("Settlement", settlement_score_icon)
        }
    };
    let score_icon = score_icon.ok_or(StartupLayoutTraceError::UnsupportedState {
        screen: SCREEN,
        detail: "evaluation score icon is unavailable",
    })?;
    let score_command = commands
        .iter()
        .find(|command| command.y == player.score_anchor.1 && command.text.contains("Score"))
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen: SCREEN,
            path: "game/evaluation/player-list/players/0/score".to_string(),
        })?;
    if score_command.text != score_text {
        return Err(StartupLayoutTraceError::RuntimeCaptionMismatch {
            screen: SCREEN,
            path: "game/evaluation/player-list/players/0/score".to_string(),
            expected: score_text,
            actual: score_command.text.clone(),
        });
    }
    let mut score_lines = gui_font(gui_fonts, score_command)
        .map(|font| command_lines(score_command, font))
        .filter(|lines| !lines.is_empty())
        .ok_or_else(|| StartupLayoutTraceError::RuntimeMissingText {
            screen: SCREEN,
            path: "game/evaluation/player-list/players/0/score".to_string(),
        })?;
    let icon_width = (score_icon.width() as i32 * gui_fonts.text.cell_height
        / score_icon.height().max(1) as i32)
        .max(1);
    let icon_advance = icon_width + gui_fonts.text.h_space;
    let mut score_rect = union_line_rect(&score_lines);
    score_rect.x -= icon_advance;
    score_rect.width = score_rect.width.saturating_add(icon_advance as u32);
    let tagged_score = format!("{{{{Ico:{score_icon_name}}}}}{score_text}");
    for line in &mut score_lines {
        line.text.clone_from(&tagged_score);
        line.rect = score_rect;
    }
    elements.push(LayoutElement {
        path: "game/evaluation/player-list/players/0/score".to_string(),
        role: "label".to_string(),
        rect: score_rect,
        visible: true,
        port_asset: None,
        caption: tagged_score,
        lines: score_lines,
    });

    elements.push(empty_element(
        "game/evaluation/player-list/scrollbar",
        "scrollbar",
        classic_layout_rect(player_list.scrollbar),
        None,
    ));

    for button in &layout.buttons {
        elements.push(required_runtime_caption_element(
            SCREEN,
            format!("game/evaluation/buttons/{}", button.index),
            "button",
            clonk_graphics::Rect::new(
                button.rect.x,
                button.rect.y,
                button.rect.w.max(0) as u32,
                button.rect.h.max(0) as u32,
            ),
            &button.caption,
            commands,
            &font,
        )?);
    }
    Ok(elements)
}

fn row_elements<'a>(
    path_prefix: &'static str,
    rect: LayoutRect,
    row_width: i32,
    row_height: i32,
    text_top_inset: i32,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Vec<LayoutElement> {
    commands
        .iter()
        .filter(|command| command_anchor_in(command, rect))
        .filter_map(|command| {
            let font = font(command)?;
            let lines = command_lines(command, font);
            let first = lines.first()?;
            let row_y = first.rect.y.saturating_sub(text_top_inset);
            Some((row_y, lines))
        })
        .enumerate()
        .map(|(index, (row_y, lines))| LayoutElement {
            path: format!("{path_prefix}/{index}"),
            role: "list-item".to_string(),
            rect: layout_rect(rect.x, row_y, row_width, row_height),
            visible: true,
            port_asset: None,
            caption: lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
        })
        .collect()
}

fn grouped_row_elements<'a>(
    paths_and_command_counts: impl IntoIterator<Item = (String, usize)>,
    list_rect: LayoutRect,
    minimum_height: i32,
    commands: &[CapturedClonkText],
    font: &impl Fn(&CapturedClonkText) -> Option<&'a ClonkFont>,
) -> Vec<LayoutElement> {
    let in_list = commands
        .iter()
        .filter(|command| command_anchor_in(command, list_rect))
        .collect::<Vec<_>>();
    let mut offset: usize = 0;
    paths_and_command_counts
        .into_iter()
        .filter_map(|(path, count)| {
            let end = offset.saturating_add(count).min(in_list.len());
            let row_commands = in_list.get(offset..end)?;
            offset = end;
            let lines = row_commands
                .iter()
                .flat_map(|command| {
                    font(command)
                        .into_iter()
                        .flat_map(|font| command_lines(command, font))
                })
                .collect::<Vec<_>>();
            let first = lines.first()?;
            let last = lines.last()?;
            let y = first.rect.y.saturating_sub(1);
            let bottom = last
                .rect
                .y
                .saturating_add(last.rect.height as i32)
                .saturating_add(1);
            Some(LayoutElement {
                path,
                role: "list-item".to_string(),
                rect: layout_rect(
                    list_rect.x,
                    y,
                    list_rect.width as i32,
                    bottom.saturating_sub(y).max(minimum_height),
                ),
                visible: true,
                port_asset: None,
                caption: lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                lines,
            })
        })
        .collect()
}

/// Produces the startup root trace from the same controller geometry and
/// captured CStdFont commands used to render the frame.
///
/// Pinned C++ oracle: `src/C4StartupMainDlg.cpp:42-74,111-121`.
pub(crate) fn startup_main_trace(
    menu: &StartupMainMenu,
    participants_label: &str,
    _logo_size: (u32, u32),
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    let layout = main_menu_layout(WIDTH, HEIGHT);
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    // The comparison records the native brand allocation, while the renderer
    // aspect-fits Clonk Rust's product artwork without changing its pixels or
    // proportions. Branding is the declared port-authored node on this term.
    let (logo_x, logo_y, logo_width, logo_height) =
        (WIDTH * 30 / 31 - 384, HEIGHT / 21 - 5, 384, 128);
    let participant = menu.participants_rect(participants_label);
    let mut elements = vec![
        empty_element(
            "startup/main/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            None,
        ),
        empty_element(
            "startup/main/branding/logo",
            "image",
            layout_rect(logo_x, logo_y, logo_width, logo_height),
            Some(BRANDING),
        ),
        allocated_text_element_at_anchor(
            "startup/main/branding/version",
            "label",
            layout_rect(854, 168, 394, gui_fonts.text.line_height),
            (WIDTH * 39 / 40, HEIGHT / 18 + logo_height),
            Some(BRANDING),
            commands,
            &font,
        )?,
    ];
    for ((path, _), button) in [
        ("startup/main/buttons/start-game", "start-game"),
        ("startup/main/buttons/network-game", "network-game"),
        ("startup/main/buttons/player-selection", "player-selection"),
        ("startup/main/buttons/options", "options"),
        ("startup/main/buttons/about", "about"),
        ("startup/main/buttons/exit", "exit"),
    ]
    .into_iter()
    .zip(layout.buttons)
    {
        elements.push(text_element(
            path,
            "button",
            layout_rect(button.x, button.y, button.w, button.h),
            None,
            commands,
            &font,
        )?);
    }
    elements.push(text_element(
        "startup/main/participants",
        "label",
        layout_rect(participant.x, participant.y, participant.w, participant.h),
        None,
        commands,
        &font,
    )?);
    elements.push(allocated_text_element_at_anchor(
        "startup/main/branding/fan-project",
        "label",
        layout_rect(
            0,
            HEIGHT - gui_fonts.mini.line_height / 2,
            WIDTH,
            gui_fonts.mini.line_height,
        ),
        (
            layout.fanproject_anchor_x,
            layout.client.y + layout.client.h - gui_fonts.mini.line_height / 2,
        ),
        Some(BRANDING),
        commands,
        &font,
    )?);
    trace("startup-main", elements)
}

/// Produces the initial scenario-book trace from its pixel-parity layout and
/// the live text commands emitted by the selector render.
///
/// Pinned C++ oracle: `src/C4StartupScenSelDlg.cpp:1302-1382`.
pub(crate) fn startup_scenario_selection_trace(
    list_scrollbar_visible: bool,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_scensel::BookFontSet,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    use clonk_graphics::clonk_font::ClonkFontRole;

    let layout = clonk_frontend::startup_scensel::scen_sel_layout(WIDTH, HEIGHT, gui_fonts);
    let font = |command: &CapturedClonkText| match command.role {
        ClonkFontRole::BookTitle => Some(&book_fonts.title),
        ClonkFontRole::BookCaption => Some(&book_fonts.caption),
        ClonkFontRole::BookText => Some(&book_fonts.text),
        ClonkFontRole::BookSmall => Some(&book_fonts.small),
        _ => gui_font(gui_fonts, command),
    };
    let map = layout.map_sheet;
    let search = layout.search_label;
    let edit = layout.search_edit;
    let list = layout.list;
    let list_rect = layout_rect(list.x, list.y, list.w, list.h);
    let list_client = layout_rect(list.x + 3, list.y + 3, list.w - 6 - 16, list.h - 6);
    let selection = layout.selection_info;
    let selection_rect = layout_rect(selection.x, selection.y, selection.w, selection.h);
    let back = layout.back_button;
    let open = layout.open_button;
    let checkbox = layout.user_change_checkbox;
    let fair_crew = layout.fair_crew_button;
    let record = layout.record_button;

    let mut elements = vec![
        empty_element(
            "startup/scenario-selection/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            Some(SUPER_RESOLVED_STARTUP_ART),
        ),
        label_at_anchor(
            "startup/scenario-selection/title",
            layout.title_anchor,
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/scenario-selection/book",
            "tabular",
            layout_rect(map.x, map.y, map.w, map.h),
            None,
        ),
        label_at_anchor(
            "startup/scenario-selection/book/caption",
            layout.caption_anchor,
            None,
            commands,
            &font,
        )?,
        text_element_at_anchor(
            "startup/scenario-selection/search/label",
            "label",
            layout_rect(search.x, search.y, search.w, search.h),
            (
                search.x + search.w / 2,
                search.y + (search.h - gui_fonts.text.line_height) / 2 - 1,
            ),
            None,
            commands,
            &font,
        )?,
        optional_text_element(
            "startup/scenario-selection/search/edit",
            "edit",
            layout_rect(edit.x, edit.y, edit.w, edit.h),
            None,
            commands,
            &font,
        ),
        empty_element(
            "startup/scenario-selection/list",
            "listbox",
            list_rect,
            None,
        ),
    ];
    elements.extend(row_elements(
        "startup/scenario-selection/list/items",
        list_client,
        list.w - 6 - 16,
        clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text),
        2,
        commands,
        &font,
    ));
    let scrollbar = layout.list_scrollbar;
    let mut list_scrollbar = empty_element(
        "startup/scenario-selection/list/scrollbar",
        "scrollbar",
        layout_rect(scrollbar.x, scrollbar.y, scrollbar.w, scrollbar.h),
        None,
    );
    list_scrollbar.visible = list_scrollbar_visible;
    elements.extend([
        list_scrollbar,
        optional_text_element(
            "startup/scenario-selection/selection-info",
            "text-window",
            selection_rect,
            None,
            commands,
            &font,
        ),
        text_element(
            "startup/scenario-selection/buttons/back",
            "button",
            layout_rect(back.x, back.y, back.w, back.h),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/scenario-selection/definitions",
            "checkbox",
            layout_rect(checkbox.x, checkbox.y, checkbox.w, checkbox.h),
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/scenario-selection/buttons/fair-crew",
            "icon-button",
            layout_rect(fair_crew.x, fair_crew.y, fair_crew.w, fair_crew.h),
            None,
        ),
        empty_element(
            "startup/scenario-selection/buttons/record",
            "icon-button",
            layout_rect(record.x, record.y, record.w, record.h),
            None,
        ),
        text_element(
            "startup/scenario-selection/buttons/open",
            "button",
            layout_rect(open.x, open.y, open.w, open.h),
            None,
            commands,
            &font,
        )?,
    ]);
    trace("startup-scenario-selection", elements)
}

/// Produces the game-list page of the startup network browser from the live
/// controller, native layout, and captured CStdFont commands.
///
/// Pinned C++ oracle: `src/C4StartupNetDlg.cpp:631-728`.
pub(crate) fn startup_network_browser_trace(
    controller: &clonk_frontend::startup_netdlg::NetDlgController,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    if controller.is_chat_mode() {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen: "startup-network-browser",
            detail: "the browser capture requires the game-list page",
        });
    }
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(gui_fonts);
    let layout = clonk_frontend::startup_netdlg::net_dlg_layout(WIDTH, HEIGHT, &metrics);
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    let games = layout.btn_game_list;
    let chat = layout.btn_chat;
    let internet = layout.btn_internet;
    let record = layout.btn_record;
    let caption = layout.game_list_caption;
    let list = layout.game_list;
    let viewport = layout.list_viewport;
    let scrollbar = layout.list_scrollbar;
    let ip = layout.ip_label;
    let edit = layout.join_edit;
    let tabular_top = caption.y;
    let tabular_bottom = edit.y.saturating_add(edit.h);
    let tabular_left = caption.x.min(ip.x);
    let tabular_right = caption
        .x
        .saturating_add(caption.w)
        .max(edit.x.saturating_add(edit.w));
    let mut elements = vec![
        empty_element(
            "startup/network-browser/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            Some(SUPER_RESOLVED_STARTUP_ART),
        ),
        label_at_anchor(
            "startup/network-browser/title",
            layout.title_anchor,
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/network-browser/mode/games",
            "icon-button",
            layout_rect(games.x, games.y, games.w, games.h),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/network-browser/mode/chat",
            "icon-button",
            layout_rect(chat.x, chat.y, chat.w, chat.h),
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/network-browser/content",
            "tabular",
            layout_rect(
                tabular_left,
                tabular_top,
                tabular_right.saturating_sub(tabular_left),
                tabular_bottom.saturating_sub(tabular_top),
            ),
            None,
        ),
        text_element_at_anchor(
            "startup/network-browser/content/caption",
            "label",
            layout_rect(caption.x, caption.y, caption.w, caption.h),
            (
                caption.x + 5,
                caption.y + (caption.h - gui_fonts.text.line_height) / 2 - 1,
            ),
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/network-browser/list",
            "listbox",
            layout_rect(list.x, list.y, list.w, list.h),
            None,
        ),
    ];
    let collapsed_line_limit = if controller.list_is_collapsed() { 2 } else { 5 };
    let mut row_counts = Vec::new();
    if controller.masterserver_signup() {
        row_counts.push((
            "startup/network-browser/list/masterserver".to_string(),
            (2 + controller.masterserver_entry().extra_lines.len()).min(collapsed_line_limit),
        ));
    }
    row_counts.extend(controller.games().iter().enumerate().map(|(index, game)| {
        (
            format!("startup/network-browser/list/games/{index}"),
            (2 + game.extra_lines.len()).min(collapsed_line_limit),
        )
    }));
    elements.extend(grouped_row_elements(
        row_counts,
        layout_rect(viewport.x, viewport.y, viewport.w, viewport.h),
        layout.list_entry.h,
        commands,
        &font,
    ));
    elements.extend([
        LayoutElement {
            visible: controller.list_max_scroll() > 0,
            ..empty_element(
                "startup/network-browser/list/scrollbar",
                "scrollbar",
                layout_rect(scrollbar.x, scrollbar.y, scrollbar.w, scrollbar.h),
                None,
            )
        },
        text_element_at_anchor(
            "startup/network-browser/join/label",
            "label",
            layout_rect(ip.x, ip.y, ip.w, ip.h),
            (
                ip.x + ip.w / 2,
                ip.y + (ip.h - gui_fonts.text.line_height) / 2 - 1,
            ),
            None,
            commands,
            &font,
        )?,
        optional_text_element(
            "startup/network-browser/join/address",
            "edit",
            layout_rect(edit.x, edit.y, edit.w, edit.h),
            None,
            commands,
            &font,
        ),
        text_element(
            "startup/network-browser/config/internet",
            "icon-button",
            layout_rect(internet.x, internet.y, internet.w, internet.h),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/network-browser/config/record",
            "icon-button",
            layout_rect(record.x, record.y, record.w, record.h),
            None,
            commands,
            &font,
        )?,
    ]);
    for (path, button) in [
        "startup/network-browser/buttons/back",
        "startup/network-browser/buttons/reload",
        "startup/network-browser/buttons/join",
        "startup/network-browser/buttons/new-game",
    ]
    .into_iter()
    .zip(layout.buttons)
    {
        elements.push(text_element(
            path,
            "button",
            layout_rect(button.x, button.y, button.w, button.h),
            None,
            commands,
            &font,
        )?);
    }
    trace("startup-network-browser", elements)
}

/// Produces the live player- or crew-selection projection. The acquisition
/// case uses player mode; crew mode remains deterministic and uses its four
/// actually visible buttons rather than fabricating hidden controls.
///
/// Pinned C++ oracle: `src/C4StartupPlrSelDlg.cpp:545-583,636-673`.
pub(crate) fn startup_player_selection_trace(
    controller: &clonk_frontend::startup_plrsel::PlrSelController,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_plrsel::BookFontSet,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    use clonk_graphics::clonk_font::ClonkFontRole;

    let layout = controller.layout();
    let expected = clonk_frontend::startup_plrsel::plrsel_layout_with_fonts(
        WIDTH, HEIGHT, gui_fonts, book_fonts,
    );
    if layout != expected {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen: "startup-player-selection",
            detail: "controller has not been laid out at 1280x720 with the live fonts",
        });
    }
    let font = |command: &CapturedClonkText| match command.role {
        ClonkFontRole::BookCaption => Some(&book_fonts.caption),
        ClonkFontRole::BookText => Some(&book_fonts.text),
        _ => gui_font(gui_fonts, command),
    };
    let list = layout.plr_list;
    let viewport = layout.list_viewport;
    let scrollbar = layout.list_scrollbar;
    let info = layout.info_window;
    let picture = layout.picture_area;
    let mut elements = vec![
        empty_element(
            "startup/player-selection/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            Some(SUPER_RESOLVED_STARTUP_ART),
        ),
        label_at_anchor(
            "startup/player-selection/title",
            layout.title_anchor,
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/player-selection/list",
            "listbox",
            layout_rect(list.x, list.y, list.w, list.h),
            None,
        ),
    ];
    let list_commands = commands
        .iter()
        .filter(|command| {
            command_anchor_in(
                command,
                layout_rect(viewport.x, viewport.y, viewport.w, viewport.h),
            )
        })
        .collect::<Vec<_>>();
    for index in 0..controller.row_count() {
        let row_y = viewport
            .y
            .saturating_add(layout.item_pitch.saturating_mul(index as i32))
            .saturating_sub(controller.list_scroll_offset());
        if row_y >= viewport.y.saturating_add(viewport.h)
            || row_y.saturating_add(layout.item_height) <= viewport.y
        {
            continue;
        }
        let row_rect = layout_rect(viewport.x, row_y, layout.item_width, layout.item_height);
        let lines = list_commands
            .iter()
            .filter(|command| command_anchor_in(command, row_rect))
            .flat_map(|command| {
                font(command)
                    .into_iter()
                    .flat_map(|font| command_lines(command, font))
            })
            .collect::<Vec<_>>();
        let base = format!("startup/player-selection/list/items/{index}");
        elements.push(LayoutElement {
            path: base.clone(),
            role: "list-item".to_string(),
            rect: row_rect,
            visible: true,
            port_asset: None,
            caption: lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines: lines.clone(),
        });
        elements.push(empty_element(
            format!("{base}/active"),
            "checkbox",
            layout_rect(viewport.x, row_y, layout.item_height, layout.item_height),
            None,
        ));
        elements.push(empty_element(
            format!("{base}/icon"),
            "picture",
            layout_rect(
                viewport.x + layout.item_height + 2,
                row_y,
                layout.item_height,
                layout.item_height,
            ),
            None,
        ));
        if !lines.is_empty() {
            let label_x = viewport.x + (layout.item_height + 2) * 2;
            elements.push(LayoutElement {
                path: format!("{base}/name"),
                role: "label".to_string(),
                rect: layout_rect(
                    label_x,
                    row_y + 2,
                    viewport.x + layout.item_width - label_x - 2,
                    book_fonts.text.line_height,
                ),
                visible: true,
                port_asset: None,
                caption: lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                lines,
            });
        }
    }
    elements.extend([
        LayoutElement {
            visible: controller.list_max_scroll() > 0,
            ..empty_element(
                "startup/player-selection/list/scrollbar",
                "scrollbar",
                layout_rect(scrollbar.x, scrollbar.y, scrollbar.w, scrollbar.h),
                None,
            )
        },
        optional_text_element(
            "startup/player-selection/selection-info",
            "text-window",
            layout_rect(info.x, info.y, info.w, info.h),
            None,
            commands,
            &font,
        ),
        empty_element(
            "startup/player-selection/portrait",
            "picture",
            layout_rect(picture.x, picture.y, picture.w, picture.h),
            None,
        ),
    ]);
    let (paths, buttons): (&[&str], &[_]) = if controller.is_crew_mode() {
        (
            &[
                "startup/player-selection/buttons/back",
                "startup/player-selection/buttons/activate",
                "startup/player-selection/buttons/delete",
                "startup/player-selection/buttons/rename",
            ],
            &layout.crew_buttons,
        )
    } else {
        (
            &[
                "startup/player-selection/buttons/back",
                "startup/player-selection/buttons/new-player",
                "startup/player-selection/buttons/activate",
                "startup/player-selection/buttons/delete",
                "startup/player-selection/buttons/properties",
                "startup/player-selection/buttons/crew",
            ],
            &layout.buttons,
        )
    };
    for (path, button) in paths.iter().copied().zip(buttons.iter()) {
        elements.push(text_element(
            path,
            "button",
            layout_rect(button.x, button.y, button.w, button.h),
            None,
            commands,
            &font,
        )?);
    }
    trace("startup-player-selection", elements)
}

/// Produces the initial Program sheet from the live options state, its native
/// 1280x720 layout, and the text commands actually submitted by the renderer.
///
/// Pinned C++ oracle: `src/C4StartupOptionsDlg.cpp:609-792,1039` and
/// `src/gui/C4GuiTabular.cpp:380-462`.
pub(crate) fn startup_options_trace(
    state: &clonk_frontend::startup_options_dlg::OptionsDlgState,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_options_dlg::BookFonts,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    use clonk_frontend::startup_options_controls::ControlDevice;
    use clonk_frontend::startup_options_dlg::OptionsSheet;
    use clonk_graphics::clonk_font::ClonkFontRole;

    if state.active_sheet() != OptionsSheet::Program {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen: "startup-options",
            detail: "the acquisition capture requires the initial Program sheet",
        });
    }
    let layout = clonk_frontend::startup_options_dlg::options_dlg_layout_for(
        WIDTH,
        HEIGHT,
        gui_fonts,
        book_fonts,
        state.labels(),
        state.controls().visible_sets(ControlDevice::Gamepad),
    );
    let font = |command: &CapturedClonkText| match command.role {
        ClonkFontRole::BookTitle => Some(&book_fonts.book_title),
        ClonkFontRole::BookCaption => Some(&book_fonts.book_caption),
        ClonkFontRole::BookText => Some(&book_fonts.book),
        ClonkFontRole::BookSmall => Some(&book_fonts.book_small),
        _ => gui_font(gui_fonts, command),
    };
    macro_rules! native_rect {
        ($value:expr) => {{
            let value = $value;
            layout_rect(value.x, value.y, value.w, value.h)
        }};
    }
    let mut elements = vec![
        empty_element(
            "startup/options/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            None,
        ),
        text_element_at_anchor(
            "startup/options/title",
            "label",
            native_rect!(layout.title_label),
            layout.title_center,
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/buttons/back",
            "button",
            native_rect!(layout.back_button),
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/options/tabs",
            "tabular",
            native_rect!(layout.tabular),
            None,
        ),
        empty_element(
            "startup/options/tabs/paper",
            "image",
            native_rect!(layout.paper),
            Some(SUPER_RESOLVED_STARTUP_ART),
        ),
    ];
    for (index, id) in [
        "program", "graphics", "sound", "keyboard", "gamepad", "network",
    ]
    .into_iter()
    .enumerate()
    {
        let (x, y) = layout.tab_clips[index];
        let path = match id {
            "program" => "startup/options/tabs/program",
            "graphics" => "startup/options/tabs/graphics",
            "sound" => "startup/options/tabs/sound",
            "keyboard" => "startup/options/tabs/keyboard",
            "gamepad" => "startup/options/tabs/gamepad",
            "network" => "startup/options/tabs/network",
            _ => unreachable!(),
        };
        elements.push(text_element_at_anchor(
            path,
            "tab",
            layout_rect(x, y, 120, 80),
            layout.tab_captions[index],
            None,
            commands,
            &font,
        )?);
    }

    let group = layout.group;
    elements.extend([
        label_at_anchor(
            "startup/options/program/language/label",
            layout.language_label,
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/language/combo",
            "combo-box",
            native_rect!(layout.language_combo),
            None,
            commands,
            &font,
        )?,
        label_at_anchor(
            "startup/options/program/language/info",
            layout.language_info,
            None,
            commands,
            &font,
        )?,
        label_at_anchor(
            "startup/options/program/font/label",
            layout.font_label,
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/font/face",
            "combo-box",
            native_rect!(layout.font_face_combo),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/font/size",
            "combo-box",
            native_rect!(layout.font_size_combo),
            None,
            commands,
            &font,
        )?,
        label_at_anchor(
            "startup/options/program/white-chat/label",
            layout.white_chat_label,
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/white-chat/ingame",
            "checkbox",
            native_rect!(layout.ingame_check),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/white-chat/lobby",
            "checkbox",
            native_rect!(layout.lobby_check),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/timestamps",
            "checkbox",
            native_rect!(layout.timestamps_check),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/preloading",
            "checkbox",
            native_rect!(layout.preloading_check),
            None,
            commands,
            &font,
        )?,
        text_element_at_anchor(
            "startup/options/program/fair-crew",
            "group-box",
            native_rect!(group),
            (group.x + 9, group.y),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/fair-crew/weak",
            "label",
            native_rect!(layout.weak_label),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/fair-crew/strong",
            "label",
            native_rect!(layout.strong_label),
            None,
            commands,
            &font,
        )?,
        empty_element(
            "startup/options/program/fair-crew/slider",
            "scrollbar",
            native_rect!(layout.slider),
            None,
        ),
        text_element(
            "startup/options/program/reset",
            "button",
            native_rect!(layout.reset_button),
            None,
            commands,
            &font,
        )?,
        text_element(
            "startup/options/program/advanced",
            "button",
            native_rect!(layout.advanced_button),
            None,
            commands,
            &font,
        )?,
    ]);
    trace("startup-options", elements)
}

/// Produces the initial Credits page from the live About state and the text
/// commands actually emitted by its renderer.
///
/// Pinned C++ oracle: `src/C4StartupAboutDlg.cpp:262-301,325-350`.
pub(crate) fn startup_about_trace(
    state: &clonk_frontend::startup_about_dlg::AboutDlgState,
    commands: &[CapturedClonkText],
    gui_fonts: &clonk_frontend::ClonkFontSet,
) -> Result<LayoutTrace, StartupLayoutTraceError> {
    use clonk_frontend::startup_about_dlg::{AboutPage, CREDITS_SECTIONS};

    if state.current_page() != AboutPage::Credits {
        return Err(StartupLayoutTraceError::UnsupportedState {
            screen: "startup-about",
            detail: "the acquisition capture requires the initial Credits page",
        });
    }
    let layout = clonk_frontend::startup_about_dlg::about_layout(WIDTH, HEIGHT);
    let font = |command: &CapturedClonkText| gui_font(gui_fonts, command);
    macro_rules! native_rect {
        ($value:expr) => {{
            let value = $value;
            layout_rect(value.x, value.y, value.w, value.h)
        }};
    }
    let mut elements = vec![
        empty_element(
            "startup/about/background",
            "image",
            layout_rect(0, 0, WIDTH, HEIGHT),
            None,
        ),
        label_at_anchor(
            "startup/about/title",
            layout.title_anchor,
            None,
            commands,
            &font,
        )?,
        allocated_text_element_at_anchor(
            "startup/about/branding/fan-project",
            "label",
            layout_rect(
                0,
                HEIGHT - gui_fonts.mini.line_height / 2,
                WIDTH,
                gui_fonts.mini.line_height,
            ),
            layout.fanproject_anchor,
            Some(BRANDING),
            commands,
            &font,
        )?,
    ];
    for (path, button) in [
        "startup/about/buttons/back",
        "startup/about/buttons/check-updates",
        "startup/about/buttons/licenses",
    ]
    .into_iter()
    .zip(layout.buttons)
    {
        elements.push(text_element(
            path,
            "button",
            native_rect!(button),
            None,
            commands,
            &font,
        )?);
    }
    for (index, (id, section)) in [
        "game-design",
        "engine-and-tools",
        "scripting",
        "additional-art",
        "music",
        "voice",
        "web",
    ]
    .into_iter()
    .zip(layout.sections)
    .enumerate()
    {
        let caption_path = match id {
            "game-design" => "startup/about/credits/game-design/caption",
            "engine-and-tools" => "startup/about/credits/engine-and-tools/caption",
            "scripting" => "startup/about/credits/scripting/caption",
            "additional-art" => "startup/about/credits/additional-art/caption",
            "music" => "startup/about/credits/music/caption",
            "voice" => "startup/about/credits/voice/caption",
            "web" => "startup/about/credits/web/caption",
            _ => unreachable!(),
        };
        let text_path = match id {
            "game-design" => "startup/about/credits/game-design/text",
            "engine-and-tools" => "startup/about/credits/engine-and-tools/text",
            "scripting" => "startup/about/credits/scripting/text",
            "additional-art" => "startup/about/credits/additional-art/text",
            "music" => "startup/about/credits/music/text",
            "voice" => "startup/about/credits/voice/text",
            "web" => "startup/about/credits/web/text",
            _ => unreachable!(),
        };
        elements.push(label_at_anchor(
            caption_path,
            section.caption_pos,
            None,
            commands,
            &font,
        )?);
        elements.push(optional_text_element(
            text_path,
            "text-window",
            native_rect!(section.textbox),
            None,
            commands,
            &font,
        ));

        // CustomMarginTextWindow<0,8,0,8> reserves the native 16px
        // ScrollWindow column even when its auto-scrollbar is hidden. The
        // public SectionLayout plus the live font/section length therefore
        // determine both its exact rect and visibility without reading pixels.
        let viewport_height = (section.textbox.h - 16).max(0);
        let line_count = CREDITS_SECTIONS[index].1.len() as i32;
        let content_height = (gui_fonts.text.line_height * line_count
            + (gui_fonts.text.line_height / 3) * (line_count - 1))
            .max(5);
        let scrollbar_path = format!("startup/about/credits/{id}/scrollbar");
        elements.push(LayoutElement {
            visible: content_height > viewport_height,
            ..empty_element(
                scrollbar_path,
                "scrollbar",
                layout_rect(
                    section.textbox.x + section.textbox.w - 16,
                    section.textbox.y + 8,
                    16,
                    viewport_height,
                ),
                None,
            )
        });
    }
    trace("startup-about", elements)
}

pub(crate) fn serialize_layout_trace(
    trace: &LayoutTrace,
) -> Result<String, StartupLayoutTraceError> {
    serde_json::to_string(trace)
        .map_err(|error| StartupLayoutTraceError::Serialization(error.to_string()))
}

#[cfg(test)]
fn startup_main_trace_for_logo_size(logo_size: (u32, u32)) -> LayoutTrace {
    use clonk_frontend::clonk_fonts::build_font_set;
    use clonk_graphics::{BitmapFont, PixelFormat, Surface};
    use std::sync::Arc;

    let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
    let fonts = Arc::new(build_font_set(bytes).expect("build pinned startup font"));
    let mut menu = StartupMainMenu::new(Arc::new(BitmapFont::new()), None);
    menu.set_clonk_fonts(Some(Arc::clone(&fonts)));
    menu.resize(WIDTH as f32, HEIGHT as f32);
    let participants = "Players: Ada";
    let mut surface = Surface::new(WIDTH as u32, HEIGHT as u32, PixelFormat::Rgba8888);
    surface.begin_clonk_text_capture();
    menu.render(&mut surface, participants);
    let logo_height = 128;
    fonts.text.draw(
        &mut surface,
        WIDTH * 39 / 40,
        HEIGHT / 18 + logo_height,
        "Version 0.20.2",
        [255, 255, 255, 255],
        TextAlign::Right,
        true,
    );
    let commands = surface.take_clonk_text_capture();
    startup_main_trace(&menu, participants, logo_size, &commands, &fonts)
        .expect("main trace from live controller render")
}

#[cfg(test)]
fn startup_main_trace_for_test() -> LayoutTrace {
    startup_main_trace_for_logo_size((960, 320))
}

#[cfg(test)]
fn test_text_command(
    role: clonk_graphics::clonk_font::ClonkFontRole,
    x: i32,
    y: i32,
    text: impl Into<String>,
    align: TextAlign,
) -> CapturedClonkText {
    CapturedClonkText {
        role,
        x,
        y,
        text: text.into(),
        color: [255; 4],
        align,
        markup: true,
        clip: None,
        gamma: None,
        images: Vec::new(),
        zoom: 1.0,
    }
}

#[cfg(test)]
fn startup_scenario_selection_trace_for_test(
    gui: &clonk_frontend::ClonkFontSet,
    book: &clonk_frontend::startup_scensel::BookFontSet,
) -> LayoutTrace {
    use clonk_graphics::clonk_font::ClonkFontRole;

    let layout = clonk_frontend::startup_scensel::scen_sel_layout(WIDTH, HEIGHT, gui);
    let mut commands = vec![
        test_text_command(
            ClonkFontRole::GuiTitle,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "Scenario selection",
            TextAlign::Center,
        ),
        test_text_command(
            ClonkFontRole::BookTitle,
            layout.caption_anchor.0,
            layout.caption_anchor.1,
            "Missions",
            TextAlign::Center,
        ),
        test_text_command(
            ClonkFontRole::BookText,
            layout.search_label.x + layout.search_label.w / 2,
            layout.search_label.y + (layout.search_label.h - gui.text.line_height) / 2 - 1,
            "Search:",
            TextAlign::Center,
        ),
    ];
    for (rect, role, text) in [
        (layout.back_button, ClonkFontRole::GuiCaption, "Back"),
        (
            layout.user_change_checkbox,
            ClonkFontRole::BookText,
            "Definitions",
        ),
        (layout.open_button, ClonkFontRole::GuiCaption, "Open"),
    ] {
        commands.push(test_text_command(
            role,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    startup_scenario_selection_trace(false, &commands, gui, book)
        .expect("scenario-selection trace from native layout")
}

#[cfg(test)]
fn startup_network_browser_trace_for_test(gui: &clonk_frontend::ClonkFontSet) -> LayoutTrace {
    use clonk_frontend::startup_netdlg::{NetDlgConfig, NetDlgController, NetDlgFontMetrics};
    use clonk_graphics::clonk_font::ClonkFontRole;

    let metrics = NetDlgFontMetrics::from_fonts(gui);
    let layout = clonk_frontend::startup_netdlg::net_dlg_layout(WIDTH, HEIGHT, &metrics);
    let mut controller = NetDlgController::new(NetDlgConfig::default(), metrics);
    controller.set_text_font(&gui.text);
    controller.resize(WIDTH, HEIGHT);
    let mut commands = vec![
        test_text_command(
            ClonkFontRole::GuiTitle,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "Network games",
            TextAlign::Center,
        ),
        test_text_command(
            ClonkFontRole::GuiCaption,
            layout.game_list_caption.x + 5,
            layout.game_list_caption.y + (layout.game_list_caption.h - gui.text.line_height) / 2
                - 1,
            "Game list",
            TextAlign::Left,
        ),
        test_text_command(
            ClonkFontRole::GuiText,
            layout.ip_label.x + layout.ip_label.w / 2,
            layout.ip_label.y + (layout.ip_label.h - gui.text.line_height) / 2 - 1,
            "Address:",
            TextAlign::Center,
        ),
    ];
    for (rect, role, text) in [
        (layout.btn_game_list, ClonkFontRole::GuiText, "Games"),
        (layout.btn_chat, ClonkFontRole::GuiText, "Chat"),
        (layout.btn_internet, ClonkFontRole::GuiText, "Internet"),
        (layout.btn_record, ClonkFontRole::GuiText, "Record"),
    ] {
        commands.push(test_text_command(
            role,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    for (rect, text) in layout
        .buttons
        .into_iter()
        .zip(["Back", "Reload", "Join", "New game"])
    {
        commands.push(test_text_command(
            ClonkFontRole::GuiCaption,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    for (index, text) in ["Master server", "Querying..."].into_iter().enumerate() {
        commands.push(test_text_command(
            ClonkFontRole::GuiText,
            layout.list_viewport.x + layout.list_entry.h + 3,
            layout.list_viewport.y + 1 + index as i32 * (gui.text.line_height + 2),
            text,
            TextAlign::Left,
        ));
    }
    startup_network_browser_trace(&controller, &commands, gui)
        .expect("network-browser trace from native controller")
}

#[cfg(test)]
fn startup_player_selection_trace_for_test(
    gui: &clonk_frontend::ClonkFontSet,
    book: &clonk_frontend::startup_plrsel::BookFontSet,
) -> LayoutTrace {
    use clonk_graphics::clonk_font::ClonkFontRole;

    let mut controller = clonk_frontend::startup_plrsel::PlrSelController::new(0);
    controller.resize_with_fonts(WIDTH, HEIGHT, gui, book);
    let layout = controller.layout();
    let mut commands = vec![test_text_command(
        ClonkFontRole::GuiTitle,
        layout.title_anchor.0,
        layout.title_anchor.1,
        "Player selection",
        TextAlign::Center,
    )];
    for (rect, text) in layout.buttons.into_iter().zip([
        "Back",
        "New player",
        "Activate",
        "Delete",
        "Properties",
        "Crew",
    ]) {
        commands.push(test_text_command(
            ClonkFontRole::GuiCaption,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    startup_player_selection_trace(&controller, &commands, gui, book)
        .expect("player-selection trace from native controller")
}

#[cfg(test)]
fn startup_options_trace_for_test(
    gui: &clonk_frontend::ClonkFontSet,
    book: &clonk_frontend::startup_options_dlg::BookFonts,
) -> LayoutTrace {
    use clonk_frontend::startup_options_controls::ControlDevice;
    use clonk_frontend::startup_options_dlg::{
        options_dlg_layout_for, OptionsDlgState, ProgramSheetState,
    };
    use clonk_graphics::clonk_font::ClonkFontRole;

    let mut state = OptionsDlgState::new(ProgramSheetState::default());
    state.resize(WIDTH, HEIGHT, gui, book);
    let layout = options_dlg_layout_for(
        WIDTH,
        HEIGHT,
        gui,
        book,
        state.labels(),
        state.controls().visible_sets(ControlDevice::Gamepad),
    );
    let mut commands = vec![
        test_text_command(
            ClonkFontRole::GuiTitle,
            layout.title_center.0,
            layout.title_center.1,
            "Options",
            TextAlign::Center,
        ),
        test_text_command(
            ClonkFontRole::GuiCaption,
            layout.back_button.x + layout.back_button.w / 2,
            layout.back_button.y + layout.back_button.h / 2,
            "Back",
            TextAlign::Center,
        ),
    ];
    for (index, text) in [
        "Program", "Graphics", "Sound", "Keyboard", "Gamepad", "Network",
    ]
    .into_iter()
    .enumerate()
    {
        commands.push(test_text_command(
            ClonkFontRole::BookSmall,
            layout.tab_captions[index].0,
            layout.tab_captions[index].1,
            text,
            TextAlign::Center,
        ));
    }
    for (anchor, text) in [
        (layout.language_label, "Language:"),
        (layout.language_info, "Language information"),
        (layout.font_label, "Font:"),
        (layout.white_chat_label, "White chat:"),
    ] {
        commands.push(test_text_command(
            ClonkFontRole::BookText,
            anchor.0,
            anchor.1,
            text,
            TextAlign::Left,
        ));
    }
    for (rect, text) in [
        (layout.language_combo, "US - English"),
        (layout.font_face_combo, "Endeavour"),
        (layout.font_size_combo, "14"),
        (layout.ingame_check, "In game"),
        (layout.lobby_check, "Lobby"),
        (layout.timestamps_check, "Timestamps"),
        (layout.preloading_check, "Preloading"),
        (layout.weak_label, "weak"),
        (layout.strong_label, "strong"),
        (layout.reset_button, "Reset"),
        (layout.advanced_button, "Advanced"),
    ] {
        commands.push(test_text_command(
            ClonkFontRole::BookText,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    commands.push(test_text_command(
        ClonkFontRole::BookText,
        layout.group.x + 9,
        layout.group.y,
        "Fair crew strength",
        TextAlign::Left,
    ));
    startup_options_trace(&state, &commands, gui, book)
        .expect("options trace from native controller")
}

#[cfg(test)]
fn startup_about_trace_for_test(gui: &clonk_frontend::ClonkFontSet) -> LayoutTrace {
    use clonk_frontend::startup_about_dlg::{AboutDlgState, CREDITS_SECTIONS, FANPROJECT_TEXT};
    use clonk_graphics::clonk_font::ClonkFontRole;

    let mut state = AboutDlgState::new();
    state.resize(WIDTH, HEIGHT, gui);
    let layout = clonk_frontend::startup_about_dlg::about_layout(WIDTH, HEIGHT);
    let mut commands = vec![
        test_text_command(
            ClonkFontRole::GuiTitle,
            layout.title_anchor.0,
            layout.title_anchor.1,
            "About",
            TextAlign::Center,
        ),
        test_text_command(
            ClonkFontRole::GuiMini,
            layout.fanproject_anchor.0,
            layout.fanproject_anchor.1,
            FANPROJECT_TEXT,
            TextAlign::Right,
        ),
    ];
    for (rect, text) in layout
        .buttons
        .into_iter()
        .zip(["Back", "Check for updates", "Licenses"])
    {
        commands.push(test_text_command(
            ClonkFontRole::GuiCaption,
            rect.x + rect.w / 2,
            rect.y + rect.h / 2,
            text,
            TextAlign::Center,
        ));
    }
    for (index, section) in layout.sections.into_iter().enumerate() {
        commands.push(test_text_command(
            ClonkFontRole::GuiCaption,
            section.caption_pos.0,
            section.caption_pos.1,
            CREDITS_SECTIONS[index].0,
            TextAlign::Left,
        ));
        commands.push(test_text_command(
            ClonkFontRole::GuiText,
            section.textbox.x,
            section.textbox.y + 8,
            format!("Credits section {index}"),
            TextAlign::Left,
        ));
    }
    startup_about_trace(&state, &commands, gui).expect("about trace from native controller")
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn an_empty_edit_capture_emits_no_semantic_lines() {
        // Native constructs the search CallbackEdit empty, and the capture
        // records a semantic line only for a nonempty caption
        // (src/C4StartupScenSelDlg.cpp:1340-1347; src/C4Startup.cpp:436-461).
        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned startup GUI fonts");
        let command = test_text_command(
            clonk_graphics::clonk_font::ClonkFontRole::GuiText,
            229,
            565,
            "",
            TextAlign::Left,
        );

        assert!(command_lines(&command, &fonts.text).is_empty());
    }

    #[test]
    fn startup_main_branding_trace_uses_the_native_semantic_allocations() {
        // Pinned C++ oracle: src/C4StartupMainDlg.cpp:42-44,111-121.
        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned startup GUI fonts");
        let trace = startup_main_trace_for_test();
        let element = |path| {
            trace
                .elements
                .iter()
                .find(|element| element.path == path)
                .expect("branding element")
        };
        let logo = element("startup/main/branding/logo");
        let version = element("startup/main/branding/version");
        let footer = element("startup/main/branding/fan-project");

        assert_eq!(logo.rect, layout_rect(854, 29, 384, 128));
        assert_eq!(logo.port_asset.as_deref(), Some(BRANDING));
        assert_eq!(
            version.rect,
            layout_rect(854, 168, 394, fonts.text.line_height)
        );
        assert_eq!(version.port_asset.as_deref(), Some(BRANDING));
        assert!(version.lines.iter().all(|line| line.rect == version.rect));
        assert_eq!(
            footer.rect,
            layout_rect(
                0,
                HEIGHT - fonts.mini.line_height / 2,
                WIDTH,
                fonts.mini.line_height,
            )
        );
        assert_eq!(footer.port_asset.as_deref(), Some(BRANDING));
        assert!(footer.lines.iter().all(|line| line.rect == footer.rect));
    }

    #[test]
    fn startup_main_live_logo_uses_the_classic_semantic_allocation() {
        // C++ draws its 960x320 logo at zoom 0.4 into this exact allocation
        // (src/C4StartupMainDlg.cpp:115-121). The port asset has different
        // source dimensions, but branding does not relax trace geometry.
        let logo = image::load_from_memory(include_bytes!("../../../planet/Graphics.c4g/Logo.png"))
            .expect("load live Rust logo");
        assert_eq!((logo.width(), logo.height()), (972, 440));

        let trace = startup_main_trace_for_logo_size((logo.width(), logo.height()));
        let logo = trace
            .elements
            .iter()
            .find(|element| element.path == "startup/main/branding/logo")
            .expect("logo element");

        assert_eq!(logo.rect, layout_rect(854, 29, 384, 128));
    }

    #[test]
    fn startup_about_footer_trace_uses_the_native_semantic_allocation() {
        // Pinned C++ oracle: src/C4StartupAboutDlg.cpp:276-278.
        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned startup GUI fonts");
        let trace = startup_about_trace_for_test(&fonts);
        let footer = trace
            .elements
            .iter()
            .find(|element| element.path == "startup/about/branding/fan-project")
            .expect("about footer");
        assert_eq!(
            footer.rect,
            layout_rect(
                0,
                HEIGHT - fonts.mini.line_height / 2,
                WIDTH,
                fonts.mini.line_height,
            )
        );
        assert_eq!(footer.port_asset.as_deref(), Some(BRANDING));
        assert!(footer.lines.iter().all(|line| line.rect == footer.rect));
    }

    #[test]
    fn startup_main_trace_uses_the_native_control_order() {
        // Pinned C++ oracle: src/C4StartupMainDlg.cpp:47-74 adds the six
        // buttons, participants label, and fan-project label in this order.
        let trace = startup_main_trace_for_test();
        let paths = trace
            .elements
            .iter()
            .map(|element| element.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "startup/main/background",
                "startup/main/branding/logo",
                "startup/main/branding/version",
                "startup/main/buttons/start-game",
                "startup/main/buttons/network-game",
                "startup/main/buttons/player-selection",
                "startup/main/buttons/options",
                "startup/main/buttons/about",
                "startup/main/buttons/exit",
                "startup/main/participants",
                "startup/main/branding/fan-project",
            ]
        );
    }

    #[test]
    fn all_six_startup_layout_producers_are_nonempty_unique_and_deterministic() {
        // These are the six native startup dialogs whose constructor order is
        // pinned by C4StartupMainDlg.cpp:42-74,
        // C4StartupScenSelDlg.cpp:1302-1382, C4StartupNetDlg.cpp:631-728,
        // C4StartupPlrSelDlg.cpp:545-583, C4StartupOptionsDlg.cpp:609-792,
        // and C4StartupAboutDlg.cpp:262-350.
        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let gui = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned startup GUI fonts");
        let scenario_book = clonk_frontend::startup_scensel::build_book_font_set(bytes)
            .expect("build pinned scenario book fonts");
        let player_book = clonk_frontend::startup_plrsel::build_book_font_set(bytes)
            .expect("build pinned player book fonts");
        let options_book = clonk_frontend::startup_options_dlg::build_book_fonts(bytes)
            .expect("build pinned options book fonts");
        let traces = [
            startup_main_trace_for_test(),
            startup_scenario_selection_trace_for_test(&gui, &scenario_book),
            startup_network_browser_trace_for_test(&gui),
            startup_player_selection_trace_for_test(&gui, &player_book),
            startup_options_trace_for_test(&gui, &options_book),
            startup_about_trace_for_test(&gui),
        ];

        assert_eq!(
            traces.each_ref().map(|trace| trace.screen.as_str()),
            [
                "startup-main",
                "startup-scenario-selection",
                "startup-network-browser",
                "startup-player-selection",
                "startup-options",
                "startup-about",
            ]
        );
        for trace in traces {
            assert_eq!(trace.schema, LAYOUT_TRACE_SCHEMA);
            assert_eq!(trace.resolution, RESOLUTION);
            assert_eq!(trace.scale, SCALE);
            assert!(!trace.elements.is_empty(), "{} is empty", trace.screen);
            let unique = trace
                .elements
                .iter()
                .map(|element| element.path.as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                unique.len(),
                trace.elements.len(),
                "{} has duplicate paths",
                trace.screen
            );
            let first = serialize_layout_trace(&trace).expect("serialize layout trace");
            let second = serialize_layout_trace(&trace).expect("serialize layout trace again");
            assert_eq!(first, second, "{} serialization changed", trace.screen);
        }
    }

    #[test]
    fn startup_layout_producers_preserve_semantic_control_order() {
        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let gui = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned startup GUI fonts");
        let scenario_book = clonk_frontend::startup_scensel::build_book_font_set(bytes)
            .expect("build pinned scenario book fonts");
        let player_book = clonk_frontend::startup_plrsel::build_book_font_set(bytes)
            .expect("build pinned player book fonts");
        let options_book = clonk_frontend::startup_options_dlg::build_book_fonts(bytes)
            .expect("build pinned options book fonts");
        let cases = [
            (
                startup_scenario_selection_trace_for_test(&gui, &scenario_book),
                vec![
                    "startup/scenario-selection/background",
                    "startup/scenario-selection/title",
                    "startup/scenario-selection/book",
                    "startup/scenario-selection/book/caption",
                    "startup/scenario-selection/search/label",
                    "startup/scenario-selection/search/edit",
                    "startup/scenario-selection/list",
                    "startup/scenario-selection/list/scrollbar",
                    "startup/scenario-selection/selection-info",
                    "startup/scenario-selection/buttons/back",
                    "startup/scenario-selection/definitions",
                    "startup/scenario-selection/buttons/fair-crew",
                    "startup/scenario-selection/buttons/record",
                    "startup/scenario-selection/buttons/open",
                ],
            ),
            (
                startup_network_browser_trace_for_test(&gui),
                vec![
                    "startup/network-browser/background",
                    "startup/network-browser/title",
                    "startup/network-browser/mode/games",
                    "startup/network-browser/mode/chat",
                    "startup/network-browser/content",
                    "startup/network-browser/content/caption",
                    "startup/network-browser/list",
                    "startup/network-browser/list/masterserver",
                    "startup/network-browser/list/scrollbar",
                    "startup/network-browser/join/label",
                    "startup/network-browser/join/address",
                    "startup/network-browser/config/internet",
                    "startup/network-browser/config/record",
                    "startup/network-browser/buttons/back",
                    "startup/network-browser/buttons/reload",
                    "startup/network-browser/buttons/join",
                    "startup/network-browser/buttons/new-game",
                ],
            ),
            (
                startup_player_selection_trace_for_test(&gui, &player_book),
                vec![
                    "startup/player-selection/background",
                    "startup/player-selection/title",
                    "startup/player-selection/list",
                    "startup/player-selection/list/scrollbar",
                    "startup/player-selection/selection-info",
                    "startup/player-selection/portrait",
                    "startup/player-selection/buttons/back",
                    "startup/player-selection/buttons/new-player",
                    "startup/player-selection/buttons/activate",
                    "startup/player-selection/buttons/delete",
                    "startup/player-selection/buttons/properties",
                    "startup/player-selection/buttons/crew",
                ],
            ),
            (
                startup_options_trace_for_test(&gui, &options_book),
                vec![
                    "startup/options/background",
                    "startup/options/title",
                    "startup/options/buttons/back",
                    "startup/options/tabs",
                    "startup/options/tabs/paper",
                    "startup/options/tabs/program",
                    "startup/options/tabs/graphics",
                    "startup/options/tabs/sound",
                    "startup/options/tabs/keyboard",
                    "startup/options/tabs/gamepad",
                    "startup/options/tabs/network",
                    "startup/options/program/language/label",
                    "startup/options/program/language/combo",
                    "startup/options/program/language/info",
                    "startup/options/program/font/label",
                    "startup/options/program/font/face",
                    "startup/options/program/font/size",
                    "startup/options/program/white-chat/label",
                    "startup/options/program/white-chat/ingame",
                    "startup/options/program/white-chat/lobby",
                    "startup/options/program/timestamps",
                    "startup/options/program/preloading",
                    "startup/options/program/fair-crew",
                    "startup/options/program/fair-crew/weak",
                    "startup/options/program/fair-crew/strong",
                    "startup/options/program/fair-crew/slider",
                    "startup/options/program/reset",
                    "startup/options/program/advanced",
                ],
            ),
            (
                startup_about_trace_for_test(&gui),
                vec![
                    "startup/about/background",
                    "startup/about/title",
                    "startup/about/branding/fan-project",
                    "startup/about/buttons/back",
                    "startup/about/buttons/check-updates",
                    "startup/about/buttons/licenses",
                    "startup/about/credits/game-design/caption",
                    "startup/about/credits/game-design/text",
                    "startup/about/credits/game-design/scrollbar",
                    "startup/about/credits/engine-and-tools/caption",
                    "startup/about/credits/engine-and-tools/text",
                    "startup/about/credits/engine-and-tools/scrollbar",
                    "startup/about/credits/scripting/caption",
                    "startup/about/credits/scripting/text",
                    "startup/about/credits/scripting/scrollbar",
                    "startup/about/credits/additional-art/caption",
                    "startup/about/credits/additional-art/text",
                    "startup/about/credits/additional-art/scrollbar",
                    "startup/about/credits/music/caption",
                    "startup/about/credits/music/text",
                    "startup/about/credits/music/scrollbar",
                    "startup/about/credits/voice/caption",
                    "startup/about/credits/voice/text",
                    "startup/about/credits/voice/scrollbar",
                    "startup/about/credits/web/caption",
                    "startup/about/credits/web/text",
                    "startup/about/credits/web/scrollbar",
                ],
            ),
        ];
        for (trace, expected) in cases {
            assert_eq!(
                trace
                    .elements
                    .iter()
                    .map(|element| element.path.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{}",
                trace.screen
            );
        }
    }

    #[test]
    fn runtime_base_trace_keeps_the_product_logo_as_the_only_branding_exemption() {
        // Pinned C++ oracle: C4GraphicsSystem::RecalculateViewports and
        // C4UpperBoard::Draw compose the fullscreen boards around the active
        // viewport (src/C4GraphicsSystem.cpp:352-365;
        // src/C4UpperBoard.cpp:125-180).
        use clonk_frontend::{
            ActiveViewportProjection, MessageBoardOverlay, RuntimePresentationSnapshot,
        };
        use clonk_graphics::Rect as SurfaceRect;

        let bytes = include_bytes!("../../../planet/System.c4g/Endeavour.ttf");
        let fonts = clonk_frontend::clonk_fonts::build_font_set(bytes)
            .expect("build pinned runtime GUI fonts");
        let commands = [
            test_text_command(
                clonk_graphics::clonk_font::ClonkFontRole::GuiText,
                10,
                16,
                "A Clonk",
                TextAlign::Left,
            ),
            test_text_command(
                clonk_graphics::clonk_font::ClonkFontRole::GuiText,
                1207,
                14,
                "00:00:02",
                TextAlign::Left,
            ),
        ];
        let snapshot = RuntimePresentationSnapshot {
            surface_rect: SurfaceRect::new(0, 0, 1280, 720),
            active_viewports: vec![ActiveViewportProjection {
                index: 0,
                owner: 0,
                identity: Some(1),
                is_no_owner_viewport: false,
                rect: SurfaceRect::new(280, 94, 720, 560),
                content_rect: SurfaceRect::new(320, 135, 640, 480),
                target_x: 0,
                target_y: 0,
                logical_width: 640,
                logical_height: 480,
                content_origin_x: 0.0,
                content_origin_y: 0.0,
                zoom: 1.0,
            }],
            upper_board_output_rect: Some(SurfaceRect::new(0, 0, 1280, 55)),
            upper_board_logo_slot: Some(SurfaceRect::new(539, 0, 201, 67)),
            upper_board_text_width: 63,
            message_board_output_rect: Some(SurfaceRect::new(0, 698, 1280, 22)),
            scenario_title: "A Clonk".to_owned(),
            formatted_game_time: "00:00:02".to_owned(),
            message_board: MessageBoardOverlay::default(),
            players: Vec::new(),
            viewport_overlays_visible: true,
            show_player_hud_always: true,
            level_bar_cell_width: Some(8),
            viewport_control_overlays: None,
            show_commands: true,
            show_command_keys: true,
            show_portraits: true,
        };

        let trace = runtime_base_trace("hud", &snapshot, &commands, &fonts, Vec::new())
            .expect("runtime base trace");

        assert_eq!(
            trace
                .elements
                .iter()
                .map(|element| element.path.as_str())
                .collect::<Vec<_>>(),
            [
                "game/surface",
                "game/upper-board/background",
                "game/upper-board/branding/logo",
                "game/upper-board/scenario-title",
                "game/upper-board/game-time",
                "game/viewport/0",
                "game/viewport/0/landscape",
                "game/message-board/output",
            ]
        );
        let logo = &trace.elements[2];
        assert_eq!(logo.rect, layout_rect(539, 0, 201, 67));
        assert_eq!(logo.port_asset.as_deref(), Some(BRANDING));
        assert!(trace
            .elements
            .iter()
            .enumerate()
            .all(|(index, element)| index == 2 || element.port_asset.is_none()));
    }
}
