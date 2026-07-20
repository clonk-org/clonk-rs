//! Runtime `C4Network2ClientListDlg` presentation and input state.
//!
//! Network authority remains in `lc-app`; this module only owns the classic
//! dialog, its one-second snapshot, and pointer/key actions.

use anyhow::{ensure, Result};
use lc_graphics::clonk_font::TextAlign;
use lc_graphics::{GammaRamp, Surface};
use std::cell::Cell;

use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text, draw_engine_box, draw_facet_stretch, ClassicGuiSkin, IntRect,
};
use crate::{ClonkFontSet, GuiPoint, ImageData};

const ICON_CELL: u32 = 40;
const ICON_CLOSE: u32 = 34;
const ICON_NET_WAIT: u32 = 3;
const ICON_ACTIVE: u32 = 14;
const ICON_INACTIVE: u32 = 15;
const ICON_KICK: u32 = 16;
const ICON_LOADING: u32 = 17;
const ICON_SOUND: u32 = 23;
const ICON_READY: u32 = 47;
const ICON_DISCONNECT: u32 = 49;
const ICON_NO_SOUND: u32 = 52;

#[derive(Clone, Copy)]
pub struct RuntimeClientListResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub button_highlight: &'a ImageData,
}

impl RuntimeClientListResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            self.fonts.text.line_height > 0,
            "FontRegular is not initialized"
        );
        let icon_columns = self.icons.width() / ICON_CELL;
        ensure!(
            icon_columns > 0
                && self.icons.height() >= (ICON_NO_SOUND / icon_columns + 1) * ICON_CELL,
            "GUIIcons.png cannot provide the runtime client-list icon phases"
        );
        ensure!(
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png is empty"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeClientStatusIcon {
    Loading,
    Ready,
    NetWait,
    Kick,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeConnectionRow {
    pub connection_id: u32,
    pub usage: String,
    pub protocol: String,
    pub peer_address: String,
    pub packet_loss: u32,
    pub ping_ms: i32,
    pub can_disconnect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeClientRow {
    pub client_id: i32,
    pub name: String,
    pub nick: String,
    pub host: bool,
    pub local: bool,
    pub activated: bool,
    pub observer: bool,
    pub muted: bool,
    pub has_players: bool,
    pub player_names: Vec<String>,
    pub addresses: Vec<String>,
    pub status: RuntimeClientStatusIcon,
    pub wait_ms: Option<i32>,
    pub connections: Vec<RuntimeConnectionRow>,
    pub can_moderate: bool,
}

impl RuntimeClientRow {
    pub fn label(&self) -> String {
        format!("{}:{}", self.name, self.nick)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeClientListStatus {
    pub tick: i32,
    pub behind: u32,
    pub rate: i32,
    pub presend: i32,
    pub average_control_time: i64,
}

impl std::fmt::Display for RuntimeClientListStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Tick {}, Behind {}, Rate {}, PreSend {}, ACT: {}",
            self.tick, self.behind, self.rate, self.presend, self.average_control_time
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeClientListAction {
    Close,
    OpenInfo(i32),
    CloseInfo,
    ToggleMute(i32),
    ToggleActivate(i32),
    Kick(i32),
    Disconnect { client_id: i32, connection_id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    Dialog,
    Close,
    InfoClose,
    ClientInfo(i32),
    Mute(i32),
    Activate(i32),
    Kick(i32),
    Disconnect(i32, u32),
}

#[derive(Clone, Copy)]
enum RuntimeListEntry<'a> {
    Client(&'a RuntimeClientRow),
    Connection {
        client_id: i32,
        connection: &'a RuntimeConnectionRow,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeClientListLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close_button: IntRect,
    pub options: IntRect,
    pub list: IntRect,
    pub status: IntRect,
    pub row_height: i32,
    pub icon_size: i32,
}

#[derive(Clone, Debug)]
pub struct RuntimeClientListDialog {
    caption: String,
    info_caption: String,
    options: Vec<String>,
    rows: Vec<RuntimeClientRow>,
    status: RuntimeClientListStatus,
    pointer: Option<GuiPoint>,
    pointer_capture: Option<HitTarget>,
    info_client_id: Option<i32>,
    info_only: bool,
    scroll_row: Cell<usize>,
}

impl RuntimeClientListDialog {
    pub fn new(
        caption: impl Into<String>,
        options: Vec<String>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
    ) -> Self {
        Self {
            caption: caption.into(),
            info_caption: "Client information".to_string(),
            options,
            rows,
            status,
            pointer: None,
            pointer_capture: None,
            info_client_id: None,
            info_only: false,
            scroll_row: Cell::new(0),
        }
    }

    /// Reuses the C4Network2ClientDlg-compatible detail presentation without
    /// constructing the surrounding F4 client-list dialog.
    pub fn new_info(caption: impl Into<String>, row: RuntimeClientRow) -> Self {
        let client_id = row.client_id;
        Self {
            caption: String::new(),
            info_caption: caption.into(),
            options: Vec::new(),
            rows: vec![row],
            status: RuntimeClientListStatus::default(),
            pointer: None,
            pointer_capture: None,
            info_client_id: Some(client_id),
            info_only: true,
            scroll_row: Cell::new(0),
        }
    }

    pub fn with_info_caption(mut self, caption: impl Into<String>) -> Self {
        self.info_caption = caption.into();
        self
    }

    pub fn rows(&self) -> &[RuntimeClientRow] {
        &self.rows
    }

    pub fn status(&self) -> RuntimeClientListStatus {
        self.status
    }

    pub fn status_text(&self) -> String {
        self.status.to_string()
    }

    pub fn info_client_id(&self) -> Option<i32> {
        self.info_client_id
    }

    pub const fn is_info_only(&self) -> bool {
        self.info_only
    }

    pub fn replace_snapshot(
        &mut self,
        options: Vec<String>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
    ) {
        self.options = options;
        self.rows = rows;
        self.status = status;
        if self
            .info_client_id
            .is_some_and(|id| !self.rows.iter().any(|row| row.client_id == id))
        {
            self.info_client_id = None;
        }
        self.scroll_row.set(
            self.scroll_row
                .get()
                .min(self.list_row_count().saturating_sub(1)),
        );
    }

    pub fn layout(&self, preferred: IntRect, font_line_height: i32) -> RuntimeClientListLayout {
        let width = (preferred.w * 3 / 4).max(180).min(preferred.w.max(1));
        let height = (preferred.h * 3 / 4).max(120).min(preferred.h.max(1));
        let bounds = IntRect {
            x: preferred.x + (preferred.w - width) / 2,
            y: preferred.y + (preferred.h - height) / 2,
            w: width,
            h: height,
        };
        let caption_height = (font_line_height + 8).max(24).min(height);
        let status_height = font_line_height
            .max(1)
            .min((height - caption_height).max(1));
        let client = IntRect {
            x: bounds.x + 4,
            y: bounds.y + caption_height + 3,
            w: (bounds.w - 8).max(1),
            h: (bounds.h - caption_height - status_height - 7).max(1),
        };
        let option_lines = self.options.len().clamp(1, 4) as i32;
        let option_height = (font_line_height * option_lines + 4)
            .min(client.h / 2)
            .max(font_line_height.min(client.h));
        let layout = RuntimeClientListLayout {
            bounds,
            caption: IntRect {
                x: bounds.x,
                y: bounds.y,
                w: bounds.w,
                h: caption_height,
            },
            close_button: IntRect {
                x: bounds.x + bounds.w - 20,
                y: bounds.y + (caption_height - 16) / 2,
                w: 16,
                h: 16,
            },
            options: IntRect {
                x: client.x,
                y: client.y,
                w: client.w,
                h: option_height,
            },
            list: IntRect {
                x: client.x,
                y: client.y + option_height + 2,
                w: client.w,
                h: (client.h - option_height - 2).max(1),
            },
            status: IntRect {
                x: bounds.x + 4,
                y: bounds.y + bounds.h - status_height - 3,
                w: (bounds.w - 8).max(1),
                h: status_height,
            },
            row_height: (font_line_height + 4).max(18),
            icon_size: font_line_height.max(16),
        };
        self.clamped_scroll_row(&layout);
        layout
    }

    /// Routes the native signed wheel delta over the client-list viewport.
    /// Positive deltas scroll toward the top; nonzero partial rows advance by
    /// one complete row so rendering and hit-testing never disagree.
    pub fn handle_wheel(
        &mut self,
        point: GuiPoint,
        delta: i32,
        preferred: IntRect,
        font_line_height: i32,
    ) -> bool {
        self.pointer = Some(point);
        let layout = self.layout(preferred, font_line_height);
        if self.info_client_id.is_some() || !contains(layout.list, point) {
            return false;
        }
        if delta != 0 {
            let row_height = layout.row_height.max(1) as usize;
            let row_delta = (delta.unsigned_abs() as usize)
                .saturating_add(row_height - 1)
                / row_height;
            let current = self.clamped_scroll_row(&layout);
            let next = if delta > 0 {
                current.saturating_sub(row_delta)
            } else {
                current
                    .saturating_add(row_delta)
                    .min(self.max_scroll_row(&layout))
            };
            self.scroll_row.set(next);
        }
        true
    }

    pub fn handle_pointer_move(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> bool {
        self.pointer = Some(point);
        self.pointer_capture.is_some()
            || self
                .hit_target(point, &self.layout(preferred, font_line_height))
                .is_some()
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> bool {
        self.pointer = Some(point);
        self.pointer_capture = self.hit_target(point, &self.layout(preferred, font_line_height));
        self.pointer_capture.is_some()
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> Option<RuntimeClientListAction> {
        self.pointer = Some(point);
        let pressed = self.pointer_capture.take()?;
        let released = self.hit_target(point, &self.layout(preferred, font_line_height));
        if released != Some(pressed) {
            return None;
        }
        let action = match pressed {
            HitTarget::Close => RuntimeClientListAction::Close,
            HitTarget::InfoClose => {
                self.info_client_id = None;
                RuntimeClientListAction::CloseInfo
            }
            HitTarget::ClientInfo(client_id) => {
                self.info_client_id = Some(client_id);
                RuntimeClientListAction::OpenInfo(client_id)
            }
            HitTarget::Mute(client_id) => RuntimeClientListAction::ToggleMute(client_id),
            HitTarget::Activate(client_id) => RuntimeClientListAction::ToggleActivate(client_id),
            HitTarget::Kick(client_id) => RuntimeClientListAction::Kick(client_id),
            HitTarget::Disconnect(client_id, connection_id) => {
                RuntimeClientListAction::Disconnect {
                    client_id,
                    connection_id,
                }
            }
            HitTarget::Dialog => return None,
        };
        Some(action)
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pointer_capture = None;
    }

    pub fn handle_escape(&mut self, pressed: bool) -> Option<RuntimeClientListAction> {
        if !pressed {
            return None;
        }
        if self.info_client_id.take().is_some() {
            Some(RuntimeClientListAction::CloseInfo)
        } else {
            Some(RuntimeClientListAction::Close)
        }
    }

    fn list_entries(&self) -> impl Iterator<Item = RuntimeListEntry<'_>> {
        self.rows.iter().flat_map(|row| {
            std::iter::once(RuntimeListEntry::Client(row)).chain(row.connections.iter().map(
                move |connection| RuntimeListEntry::Connection {
                    client_id: row.client_id,
                    connection,
                },
            ))
        })
    }

    fn list_row_count(&self) -> usize {
        self.rows.iter().fold(0usize, |count, row| {
            count.saturating_add(1usize.saturating_add(row.connections.len()))
        })
    }

    fn visible_list_row_count(layout: &RuntimeClientListLayout) -> usize {
        (layout.list.h - 2).max(0) as usize / layout.row_height.max(1) as usize
    }

    fn max_scroll_row(&self, layout: &RuntimeClientListLayout) -> usize {
        self.list_row_count()
            .saturating_sub(Self::visible_list_row_count(layout))
    }

    fn clamped_scroll_row(&self, layout: &RuntimeClientListLayout) -> usize {
        let scroll_row = self.scroll_row.get().min(self.max_scroll_row(layout));
        self.scroll_row.set(scroll_row);
        scroll_row
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(preferred, resources.fonts.text.line_height);
        if self.info_only {
            if let Some(client_id) = self.info_client_id {
                self.draw_client_info(surface, client_id, &layout, resources, active, gamma);
            }
            return Ok(());
        }
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        resources.skin.draw_caption_with_right_indent(
            surface,
            layout.caption,
            &self.caption,
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            22,
            gamma,
        );
        self.draw_icon_button(
            surface,
            layout.close_button,
            ICON_CLOSE,
            HitTarget::Close,
            &layout,
            resources,
            active,
            gamma,
        );

        draw_engine_box(
            surface,
            layout.options.x,
            layout.options.y,
            layout.options.x + layout.options.w - 1,
            layout.options.y + layout.options.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.options, gamma);
        for (index, option) in self.options.iter().enumerate() {
            let y = layout.options.y + 2 + index as i32 * resources.fonts.text.line_height;
            if y >= layout.options.y + layout.options.h {
                break;
            }
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                layout.options.x + 4,
                y,
                option,
                [200, 200, 200, 255],
                TextAlign::Left,
                gamma,
                layout.options,
            );
        }

        draw_engine_box(
            surface,
            layout.list.x,
            layout.list.y,
            layout.list.x + layout.list.w - 1,
            layout.list.y + layout.list.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.list, gamma);
        self.draw_rows(surface, &layout, resources, active, gamma);
        draw_clipped_text(
            surface,
            &resources.fonts.text,
            layout.status.x,
            layout.status.y,
            &self.status.to_string(),
            [255, 255, 255, 255],
            TextAlign::Left,
            gamma,
            layout.status,
        );

        if let Some(client_id) = self.info_client_id {
            self.draw_client_info(surface, client_id, &layout, resources, active, gamma);
        }
        Ok(())
    }

    fn draw_rows(
        &self,
        surface: &mut Surface,
        layout: &RuntimeClientListLayout,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let mut y = layout.list.y + 2;
        let scroll_row = self.clamped_scroll_row(layout);
        for entry in self.list_entries().skip(scroll_row) {
            if y + layout.row_height > layout.list.y + layout.list.h {
                break;
            }
            match entry {
                RuntimeListEntry::Client(row) => {
                    let status_rect = IntRect {
                        x: layout.list.x + 3,
                        y: y + (layout.row_height - layout.icon_size) / 2,
                        w: layout.icon_size,
                        h: layout.icon_size,
                    };
                    draw_icon(
                        surface,
                        status_rect,
                        resources.icons,
                        status_icon_phase(row),
                        gamma,
                    );
                    let mut right = layout.list.x + layout.list.w - 3;
                    if !row.host && row.can_moderate {
                        for (target, phase) in [
                            (HitTarget::Kick(row.client_id), ICON_KICK),
                            (
                                HitTarget::Activate(row.client_id),
                                if row.activated {
                                    ICON_ACTIVE
                                } else {
                                    ICON_INACTIVE
                                },
                            ),
                        ] {
                            right -= layout.icon_size;
                            self.draw_icon_button(
                                surface,
                                IntRect {
                                    x: right,
                                    y: status_rect.y,
                                    w: layout.icon_size,
                                    h: layout.icon_size,
                                },
                                phase,
                                target,
                                layout,
                                resources,
                                active,
                                gamma,
                            );
                            right -= 2;
                        }
                    }
                    if !row.local {
                        right -= layout.icon_size;
                        self.draw_icon_button(
                            surface,
                            IntRect {
                                x: right,
                                y: status_rect.y,
                                w: layout.icon_size,
                                h: layout.icon_size,
                            },
                            if row.muted { ICON_NO_SOUND } else { ICON_SOUND },
                            HitTarget::Mute(row.client_id),
                            layout,
                            resources,
                            active,
                            gamma,
                        );
                        right -= 2;
                    }
                    if let Some(wait_ms) = row.wait_ms {
                        let wait = format!("{wait_ms} ms");
                        let color = wait_color(wait_ms);
                        right -= 54;
                        draw_clipped_text(
                            surface,
                            &resources.fonts.text,
                            right + 54,
                            y + 2,
                            &wait,
                            color,
                            TextAlign::Right,
                            gamma,
                            IntRect {
                                x: right,
                                y,
                                w: 54,
                                h: layout.row_height,
                            },
                        );
                    }
                    draw_clipped_text(
                        surface,
                        &resources.fonts.text,
                        status_rect.x + status_rect.w + 3,
                        y + 2,
                        &row.label(),
                        [255, 255, 255, 255],
                        TextAlign::Left,
                        gamma,
                        IntRect {
                            x: status_rect.x + status_rect.w + 3,
                            y,
                            w: (right - status_rect.x - status_rect.w - 5).max(1),
                            h: layout.row_height,
                        },
                    );
                }
                RuntimeListEntry::Connection {
                    client_id,
                    connection,
                } => {
                    let mut connection_right = layout.list.x + layout.list.w - 3;
                    if connection.can_disconnect {
                        connection_right -= layout.icon_size;
                        self.draw_icon_button(
                            surface,
                            IntRect {
                                x: connection_right,
                                y: y + (layout.row_height - layout.icon_size) / 2,
                                w: layout.icon_size,
                                h: layout.icon_size,
                            },
                            ICON_DISCONNECT,
                            HitTarget::Disconnect(client_id, connection.connection_id),
                            layout,
                            resources,
                            active,
                            gamma,
                        );
                        connection_right -= 2;
                    }
                    let ping = if connection.ping_ms < 0 {
                        "???".to_string()
                    } else {
                        format!("{} ms", connection.ping_ms)
                    };
                    connection_right -= 54;
                    draw_clipped_text(
                        surface,
                        &resources.fonts.text,
                        connection_right + 54,
                        y + 2,
                        &ping,
                        [255, 255, 255, 255],
                        TextAlign::Right,
                        gamma,
                        IntRect {
                            x: connection_right,
                            y,
                            w: 54,
                            h: layout.row_height,
                        },
                    );
                    let description = format!(
                        "{}: {} ({} l{})",
                        connection.usage,
                        connection.protocol,
                        connection.peer_address,
                        connection.packet_loss
                    );
                    draw_clipped_text(
                        surface,
                        &resources.fonts.text,
                        layout.list.x + layout.icon_size * 2,
                        y + 2,
                        &description,
                        [220, 220, 220, 255],
                        TextAlign::Left,
                        gamma,
                        IntRect {
                            x: layout.list.x + layout.icon_size * 2,
                            y,
                            w: (connection_right - layout.list.x - layout.icon_size * 2).max(1),
                            h: layout.row_height,
                        },
                    );
                }
            }
            y += layout.row_height;
        }
    }

    fn draw_client_info(
        &self,
        surface: &mut Surface,
        client_id: i32,
        parent: &RuntimeClientListLayout,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let Some(row) = self.rows.iter().find(|row| row.client_id == client_id) else {
            return;
        };
        let width = (parent.bounds.w * 3 / 4).max(160).min(parent.bounds.w);
        let height = (parent.row_height * 8 + 16).max(90).min(parent.bounds.h);
        let bounds = IntRect {
            x: parent.bounds.x + (parent.bounds.w - width) / 2,
            y: parent.bounds.y + (parent.bounds.h - height) / 2,
            w: width,
            h: height,
        };
        let caption = IntRect {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: (resources.fonts.text.line_height + 8).max(24),
        };
        let close = IntRect {
            x: bounds.x + bounds.w - 20,
            y: caption.y + (caption.h - 16) / 2,
            w: 16,
            h: 16,
        };
        resources.skin.draw_dialog(surface, bounds, gamma);
        resources.skin.draw_caption_with_right_indent(
            surface,
            caption,
            &self.info_caption,
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            22,
            gamma,
        );
        self.draw_icon_button(
            surface,
            close,
            ICON_CLOSE,
            HitTarget::InfoClose,
            parent,
            resources,
            active,
            gamma,
        );
        let role = if row.host { "host" } else { "client" };
        let location = if row.local { "local" } else { "remote" };
        let activity = if row.activated { "active" } else { "inactive" };
        let mut lines = vec![
            format!("{activity}, {location}, {role}"),
            format!("{} ({})", row.label(), row.client_id),
        ];
        if !row.player_names.is_empty() {
            lines.push(format!("Players: {}", row.player_names.join(", ")));
        }
        if row.addresses.is_empty() {
            lines.push("No addresses available".to_string());
        } else {
            lines.push("Addresses:".to_string());
            lines.extend(row.addresses.iter().map(|address| format!("  {address}")));
        }
        if row.connections.is_empty() {
            lines.push("No connection details available".to_string());
        } else {
            lines.extend(row.connections.iter().map(|connection| {
                format!(
                    "{} {} {} ({} ms)",
                    connection.usage,
                    connection.protocol,
                    connection.peer_address,
                    connection.ping_ms
                )
            }));
        }
        let text = lines.join("|");
        draw_clipped_text(
            surface,
            &resources.fonts.text,
            bounds.x + 6,
            caption.y + caption.h + 5,
            &text,
            [255, 255, 255, 255],
            TextAlign::Left,
            gamma,
            IntRect {
                x: bounds.x + 4,
                y: caption.y + caption.h + 3,
                w: (bounds.w - 8).max(1),
                h: (bounds.h - caption.h - 7).max(1),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_icon_button(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        phase: u32,
        target: HitTarget,
        layout: &RuntimeClientListLayout,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let hovered = active
            && self.pointer.is_some_and(|point| {
                self.hit_target(point, layout)
                    .is_some_and(|hit| hit == target)
            });
        let pressed = hovered && self.pointer_capture == Some(target);
        if hovered && !pressed {
            draw_highlight(surface, rect, resources.button_highlight, gamma);
        }
        draw_icon(surface, rect, resources.icons, phase, gamma);
        if pressed {
            draw_highlight(surface, rect, resources.button_highlight, gamma);
        }
    }

    fn hit_target(&self, point: GuiPoint, layout: &RuntimeClientListLayout) -> Option<HitTarget> {
        if self.info_client_id.is_some() {
            let width = (layout.bounds.w * 3 / 4).max(160).min(layout.bounds.w);
            let height = (layout.row_height * 8 + 16).max(90).min(layout.bounds.h);
            let info = IntRect {
                x: layout.bounds.x + (layout.bounds.w - width) / 2,
                y: layout.bounds.y + (layout.bounds.h - height) / 2,
                w: width,
                h: height,
            };
            let close = IntRect {
                x: info.x + info.w - 20,
                y: info.y + ((layout.row_height + 4).max(24) - 16) / 2,
                w: 16,
                h: 16,
            };
            return if contains(close, point) {
                Some(HitTarget::InfoClose)
            } else if contains(info, point) {
                Some(HitTarget::Dialog)
            } else {
                None
            };
        }
        if contains(layout.close_button, point) {
            return Some(HitTarget::Close);
        }
        let mut y = layout.list.y + 2;
        let scroll_row = self.clamped_scroll_row(layout);
        for entry in self.list_entries().skip(scroll_row) {
            if y + layout.row_height > layout.list.y + layout.list.h {
                break;
            }
            match entry {
                RuntimeListEntry::Client(row) => {
                    let row_rect = IntRect {
                        x: layout.list.x + 2,
                        y,
                        w: (layout.list.w - 4).max(1),
                        h: layout.row_height,
                    };
                    let mut right = layout.list.x + layout.list.w - 3;
                    if !row.host && row.can_moderate {
                        right -= layout.icon_size;
                        let kick = IntRect {
                            x: right,
                            y: y + (layout.row_height - layout.icon_size) / 2,
                            w: layout.icon_size,
                            h: layout.icon_size,
                        };
                        if contains(kick, point) {
                            return Some(HitTarget::Kick(row.client_id));
                        }
                        right -= layout.icon_size + 2;
                        let activate = IntRect { x: right, ..kick };
                        if contains(activate, point) {
                            return Some(HitTarget::Activate(row.client_id));
                        }
                        right -= 2;
                    }
                    if !row.local {
                        right -= layout.icon_size;
                        let mute = IntRect {
                            x: right,
                            y: y + (layout.row_height - layout.icon_size) / 2,
                            w: layout.icon_size,
                            h: layout.icon_size,
                        };
                        if contains(mute, point) {
                            return Some(HitTarget::Mute(row.client_id));
                        }
                    }
                    if contains(row_rect, point) {
                        return Some(HitTarget::ClientInfo(row.client_id));
                    }
                }
                RuntimeListEntry::Connection {
                    client_id,
                    connection,
                } if connection.can_disconnect => {
                    let disconnect = IntRect {
                        x: layout.list.x + layout.list.w - 3 - layout.icon_size,
                        y: y + (layout.row_height - layout.icon_size) / 2,
                        w: layout.icon_size,
                        h: layout.icon_size,
                    };
                    if contains(disconnect, point) {
                        return Some(HitTarget::Disconnect(
                            client_id,
                            connection.connection_id,
                        ));
                    }
                }
                RuntimeListEntry::Connection { .. } => {}
            }
            y += layout.row_height;
        }
        contains(layout.bounds, point).then_some(HitTarget::Dialog)
    }
}

fn status_icon_phase(row: &RuntimeClientRow) -> u32 {
    match row.status {
        RuntimeClientStatusIcon::Loading => ICON_LOADING,
        RuntimeClientStatusIcon::Ready => ICON_READY,
        RuntimeClientStatusIcon::NetWait => ICON_NET_WAIT,
        RuntimeClientStatusIcon::Kick => ICON_KICK,
    }
}

fn wait_color(wait_ms: i32) -> [u8; 4] {
    let red = (255 - wait_ms.abs().saturating_mul(5)).clamp(0, 255) as u8;
    let green = (255 - wait_ms.saturating_mul(5)).clamp(0, 255) as u8;
    let blue = (255 + wait_ms.saturating_mul(5)).clamp(0, 255) as u8;
    [red, green, blue, 255]
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(
        surface,
        &lc_gui::Rect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        highlight,
        gamma,
    );
}

fn draw_icon(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u32,
    gamma: Option<&GammaRamp>,
) {
    let columns = (icons.width() / ICON_CELL).max(1);
    let source_x = phase % columns * ICON_CELL;
    let source_y = phase / columns * ICON_CELL;
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            ICON_CELL as f32,
            ICON_CELL as f32,
        ),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RuntimeClientRow {
        RuntimeClientRow {
            client_id: 7,
            name: "Remote".to_string(),
            nick: "Nick".to_string(),
            host: false,
            local: false,
            activated: true,
            observer: false,
            muted: false,
            has_players: true,
            player_names: vec!["Player".to_string()],
            addresses: Vec::new(),
            status: RuntimeClientStatusIcon::Ready,
            wait_ms: Some(12),
            connections: Vec::new(),
            can_moderate: true,
        }
    }

    #[test]
    fn status_uses_the_native_field_order() {
        assert_eq!(
            RuntimeClientListStatus {
                tick: 41,
                behind: 3,
                rate: 4,
                presend: 2,
                average_control_time: 40_000,
            }
            .to_string(),
            "Tick 41, Behind 3, Rate 4, PreSend 2, ACT: 40000"
        );
    }

    #[test]
    fn layout_uses_three_quarters_of_the_preferred_rectangle() {
        let dialog = RuntimeClientListDialog::new(
            "Network",
            vec!["League".to_string()],
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(
            IntRect {
                x: 20,
                y: 10,
                w: 800,
                h: 600,
            },
            20,
        );
        assert_eq!((layout.bounds.w, layout.bounds.h), (600, 450));
        assert_eq!((layout.bounds.x, layout.bounds.y), (120, 85));
    }

    #[test]
    fn client_row_and_escape_open_then_close_the_info_child() {
        let preferred = IntRect {
            x: 0,
            y: 0,
            w: 320,
            h: 200,
        };
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            vec!["Network game".to_string()],
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let point = GuiPoint::new(
            (layout.list.x + 25) as f32,
            (layout.list.y + layout.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(point, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(point, preferred, 16),
            Some(RuntimeClientListAction::OpenInfo(7))
        );
        assert_eq!(dialog.info_client_id(), Some(7));
        assert_eq!(
            dialog.handle_escape(true),
            Some(RuntimeClientListAction::CloseInfo)
        );
        assert_eq!(dialog.info_client_id(), None);
        assert_eq!(
            dialog.handle_escape(true),
            Some(RuntimeClientListAction::Close)
        );
    }

    #[test]
    fn standalone_client_info_starts_on_the_requested_row_and_closes_as_info() {
        let mut dialog = RuntimeClientListDialog::new_info("Client information", row());
        assert!(dialog.is_info_only());
        assert_eq!(dialog.info_client_id(), Some(7));
        assert_eq!(dialog.rows().len(), 1);
        assert_eq!(
            dialog.handle_escape(true),
            Some(RuntimeClientListAction::CloseInfo)
        );
        assert_eq!(dialog.info_client_id(), None);
    }

    #[test]
    fn wheel_scroll_makes_an_initially_hidden_client_actionable() {
        let preferred = IntRect {
            x: 0,
            y: 0,
            w: 320,
            h: 200,
        };
        let rows = (0..8)
            .map(|index| {
                let mut client = row();
                client.client_id = 100 + index;
                client.name = format!("Remote {index}");
                client
            })
            .collect();
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            vec!["Network game".to_string()],
            rows,
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let visible_rows = RuntimeClientListDialog::visible_list_row_count(&layout);
        assert!(visible_rows > 0 && visible_rows < dialog.rows().len());
        let hidden_client_id = dialog.rows()[visible_rows].client_id;
        let hidden_point = GuiPoint::new(
            (layout.list.x + layout.list.w - 3 - layout.icon_size / 2) as f32,
            (layout.list.y
                + 2
                + visible_rows as i32 * layout.row_height
                + layout.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(hidden_point, preferred, 16));
        assert_eq!(dialog.handle_pointer_up(hidden_point, preferred, 16), None);

        let first_row_point = GuiPoint::new(
            (layout.list.x + layout.list.w - 3 - layout.icon_size / 2) as f32,
            (layout.list.y + 2 + layout.row_height / 2) as f32,
        );
        let native_delta = layout.row_height.saturating_mul(visible_rows as i32);
        assert!(dialog.handle_wheel(first_row_point, -native_delta, preferred, 16));
        assert!(dialog.handle_pointer_down(first_row_point, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(first_row_point, preferred, 16),
            Some(RuntimeClientListAction::Kick(hidden_client_id))
        );

        assert!(dialog.handle_wheel(first_row_point, native_delta, preferred, 16));
        assert!(dialog.handle_pointer_down(first_row_point, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(first_row_point, preferred, 16),
            Some(RuntimeClientListAction::Kick(100))
        );
    }
}
