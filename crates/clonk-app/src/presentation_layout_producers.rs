//! Semantic layout producers for the six startup captures whose manifest
//! comparison term is `layout`.

use crate::presentation_layout::{
    LayoutElement, LayoutLine, LayoutRect, LayoutTrace, LAYOUT_TRACE_SCHEMA,
};
use crate::startup_main_classic_logo_geometry;
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
    let (logo_x, logo_y, logo_width, logo_height) =
        startup_main_classic_logo_geometry(WIDTH, HEIGHT);
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
}
