//! Runtime `C4Network2ClientListDlg` presentation and input state.
//!
//! Network authority remains in `clonk-app`; this module only owns the classic
//! dialog, its one-second snapshot, and pointer/key actions.

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};
use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::caption_scroll::{advance_caption_scroll, CaptionScrollState};
use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text, draw_clipped_text_with_markup, draw_engine_box,
    draw_facet_stretch, ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::context_menu::{draw_classic_tooltip, ClassicTooltipTracker};
use crate::game_lobby::{LobbyOptionKind, LobbyOptionRow};
use crate::info_dialog::{
    InfoScrollTarget, ScrollingInfoDialog, ScrollingInfoGeometry, ScrollingInfoMetrics,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode, StartupTooltip};

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
const TITLE_LEFT_INDENT: i32 = 5;
const TITLE_RIGHT_INDENT: i32 = 20;
const TITLE_SCROLL_DELAY: Duration = Duration::from_millis(3000);
const CONTEXT_HEIGHT: u32 = 16;
const SCROLLBAR_EXTENT: i32 = 16;
const LIST_BOX_MARGIN: i32 = 3;
const OPTION_ITEM_SPACING: i32 = 1;
const INFO_DIALOG_INDENT: i32 = 10;
const INFO_BUTTON_AREA_HEIGHT: i32 = 40;
const INFO_CLOSE_BUTTON_WIDTH: i32 = 140;
const INFO_CLOSE_BUTTON_HEIGHT: i32 = 32;
const STANDARD_BACKGROUND_COLOR: u32 = 0x4f3f_1a00;
const LIST_SELECTION: u32 = 0xafaf_0000;
const LIST_SELECTION_INACTIVE: u32 = 0xaf7f_7f7f;

#[derive(Clone, Copy)]
pub struct RuntimeClientListResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub tooltip_font: &'a ClonkFont,
    pub icons: &'a ImageData,
    pub button_highlight: &'a ImageData,
    pub context: &'a ImageData,
    pub scroll: &'a ImageData,
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
        ensure!(
            self.context.width() >= CONTEXT_HEIGHT * 2 && self.context.height() >= CONTEXT_HEIGHT,
            "GUIContext.png cannot provide the closed/open ComboBox arrows"
        );
        ensure!(
            self.scroll.width() == 32 && self.scroll.height() == 48,
            "GUIScroll.png must be 32x48"
        );
        ensure!(
            self.tooltip_font.line_height > 0,
            "classic tooltip font has no line height"
        );
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct StaticInfoDialogResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub button_highlight: &'a ImageData,
    pub scroll: &'a ImageData,
}

impl StaticInfoDialogResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            self.fonts.text.line_height > 0,
            "FontRegular is not initialized"
        );
        let icon_columns = self.icons.width() / ICON_CELL;
        ensure!(
            icon_columns > 0 && self.icons.height() >= (ICON_CLOSE / icon_columns + 1) * ICON_CELL,
            "GUIIcons.png cannot provide the InfoDialog close phase"
        );
        ensure!(
            self.button_highlight.width() > 0 && self.button_highlight.height() > 0,
            "GUIButtonHighlight.png is empty"
        );
        ensure!(
            self.scroll.width() == 32 && self.scroll.height() == 48,
            "GUIScroll.png must be 32x48"
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
    /// `getPingTime()`: the raw measured round trip shown by the per-client
    /// info text (src/C4Network2Dialogs.cpp:92-102).
    pub ping_ms: i32,
    /// `getLag()`: the live value shown by the connection list rows
    /// (src/C4Network2Dialogs.cpp:357-369).
    pub lag_ms: i32,
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
    /// `Game.Network.isHost() && pNetClient && !pNetClient->isReady()`
    /// (src/C4Network2Dialogs.cpp:71): only a host sees this, and only for a
    /// remote client that has not acknowledged the current network status.
    pub unacknowledged: bool,
}

impl RuntimeClientRow {
    pub fn label(&self) -> String {
        format!("{}:{}", self.name, self.nick)
    }
}

/// Localized templates used by `C4Network2ClientDlg::UpdateText`.
///
/// The network snapshot is owned by `clonk-app`, but the dialog can refresh
/// independently of that snapshot. Keep the resolved active-language strings
/// with the dialog so both the runtime F4 panel and the lobby's standalone
/// client-info panel use the same native templates on every refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeClientInfoResources {
    pub active: String,
    pub inactive: String,
    pub local: String,
    pub remote: String,
    pub host: String,
    pub client: String,
    pub format: String,
    pub addresses: String,
    pub conndata: String,
    pub connections: String,
    pub noaddresses: String,
    pub noconnections: String,
    pub unknown_id: String,
}

impl RuntimeClientInfoResources {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        active: impl Into<String>,
        inactive: impl Into<String>,
        local: impl Into<String>,
        remote: impl Into<String>,
        host: impl Into<String>,
        client: impl Into<String>,
        format: impl Into<String>,
        addresses: impl Into<String>,
        conndata: impl Into<String>,
        connections: impl Into<String>,
        noaddresses: impl Into<String>,
        noconnections: impl Into<String>,
        unknown_id: impl Into<String>,
    ) -> Self {
        Self {
            active: active.into(),
            inactive: inactive.into(),
            local: local.into(),
            remote: remote.into(),
            host: host.into(),
            client: client.into(),
            format: format.into(),
            addresses: addresses.into(),
            conndata: conndata.into(),
            connections: connections.into(),
            noaddresses: noaddresses.into(),
            noconnections: noconnections.into(),
            unknown_id: unknown_id.into(),
        }
    }
}

impl Default for RuntimeClientInfoResources {
    fn default() -> Self {
        Self::new(
            "Active",
            "Inactive",
            "local",
            "remote",
            "host",
            "client",
            "%s %s %s %s (ID #%d):%s",
            "Addresses:",
            "  Data: %s (%s, %d ms)",
            "Connections: %s: %s (%s, %d ms)",
            "Addresses: none",
            "Connections: Not connected",
            "Unknown client ID #%d.",
        )
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

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeClientListAction {
    Close,
    OpenInfo(i32),
    CloseInfo,
    OptionSelectionRequested {
        option: LobbyOptionKind,
        anchor: GuiPoint,
        minimum_width: i32,
    },
    ToggleMute(i32),
    ToggleActivate(i32),
    Kick(i32),
    Disconnect {
        client_id: i32,
        connection_id: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    Dialog,
    Close,
    InfoClose,
    InfoBottomClose,
    InfoScrollUp,
    InfoScrollDown,
    InfoScrollTrack,
    OptionRow(usize),
    OptionValue(usize),
    OptionScrollUp,
    OptionScrollDown,
    OptionScrollTrack,
    ClientInfo(i32),
    ConnectionRow(i32, u32),
    Mute(i32),
    Activate(i32),
    Kick(i32),
    Disconnect(i32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogTitle {
    Main,
    Info,
}

#[derive(Clone, Copy, Debug)]
struct TitleDrag {
    title: DialogTitle,
    pointer: GuiPoint,
    offset: (i32, i32),
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
    pub option_scrollbar: IntRect,
    pub list: IntRect,
    pub status: IntRect,
    pub option_rows: Vec<RuntimeClientOptionLayout>,
    pub row_height: i32,
    pub icon_size: i32,
    font_line_height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeClientInfoLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close_button: IntRect,
    pub bottom_close_button: Option<IntRect>,
    pub text_window: IntRect,
    pub text: IntRect,
    pub scrollbar: IntRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeClientOptionLayout {
    pub index: usize,
    pub rect: IntRect,
    pub value: IntRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeClientListFocus {
    Close,
    OptionsList,
    ClientList,
    Mute(i32),
    Activate(i32),
    Kick(i32),
    Disconnect { client_id: i32, connection_id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeClientListSelection {
    Client(i32),
    Connection { client_id: i32, connection_id: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeClientListTooltip {
    pub pointer: GuiPoint,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeClientListDialog {
    caption: String,
    info_dialog: ScrollingInfoDialog,
    info_resources: RuntimeClientInfoResources,
    caption_scroll: Cell<CaptionScrollState>,
    info_caption_scroll: Cell<CaptionScrollState>,
    options: Vec<LobbyOptionRow>,
    rows: Vec<RuntimeClientRow>,
    status: RuntimeClientListStatus,
    dialog_offset: (i32, i32),
    info_dialog_offset: (i32, i32),
    pointer: Option<GuiPoint>,
    pointer_capture: Option<HitTarget>,
    title_drag: Option<TitleDrag>,
    focus: Option<RuntimeClientListFocus>,
    keyboard_press: Option<(KeyCode, RuntimeClientListFocus)>,
    selected_entry: Option<RuntimeClientListSelection>,
    open_option: Option<LobbyOptionKind>,
    tooltip: ClassicTooltipTracker,
    info_open: bool,
    info_client_id: Option<i32>,
    info_only: bool,
    info_static: bool,
    info_close_label: String,
    option_caption_reference: String,
    option_caption_width: Cell<i32>,
    option_scroll_row: Cell<usize>,
    scroll_row: Cell<usize>,
}

impl RuntimeClientListDialog {
    pub fn new(
        caption: impl Into<String>,
        options: Vec<LobbyOptionRow>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
    ) -> Self {
        let option_caption_reference = options
            .iter()
            .find(|option| option.kind == LobbyOptionKind::RuntimeJoin)
            .map(|option| option.caption.clone())
            .unwrap_or_default();
        Self {
            caption: caption.into(),
            info_dialog: ScrollingInfoDialog::new("Client information", 10, true),
            info_resources: RuntimeClientInfoResources::default(),
            caption_scroll: Cell::new(CaptionScrollState::default()),
            info_caption_scroll: Cell::new(CaptionScrollState::default()),
            options,
            rows,
            status,
            dialog_offset: (0, 0),
            info_dialog_offset: (0, 0),
            pointer: None,
            pointer_capture: None,
            title_drag: None,
            focus: None,
            keyboard_press: None,
            selected_entry: None,
            open_option: None,
            tooltip: ClassicTooltipTracker::new(),
            info_open: false,
            info_client_id: None,
            info_only: false,
            info_static: false,
            info_close_label: String::new(),
            option_caption_reference,
            option_caption_width: Cell::new(0),
            option_scroll_row: Cell::new(0),
            scroll_row: Cell::new(0),
        }
    }

    /// Reuses the C4Network2ClientDlg-compatible detail presentation without
    /// constructing the surrounding F4 client-list dialog.
    ///
    /// The C++ dialog is constructed from an id alone and resolves the client
    /// in `UpdateText`, so `row` may legitimately be absent: the dialog then
    /// shows the unknown-id line (src/C4Network2Dialogs.cpp:42-59).
    pub fn new_info(
        caption: impl Into<String>,
        client_id: i32,
        row: Option<RuntimeClientRow>,
    ) -> Self {
        let rows = row.into_iter().collect::<Vec<_>>();
        let info_resources = RuntimeClientInfoResources::default();
        let lines = client_info_lines_for(&rows, client_id, &info_resources);
        Self {
            caption: String::new(),
            info_dialog: ScrollingInfoDialog::new(caption, 10, true),
            info_resources,
            caption_scroll: Cell::new(CaptionScrollState::default()),
            info_caption_scroll: Cell::new(CaptionScrollState::default()),
            options: Vec::new(),
            rows,
            status: RuntimeClientListStatus::default(),
            dialog_offset: (0, 0),
            info_dialog_offset: (0, 0),
            pointer: None,
            pointer_capture: None,
            title_drag: None,
            focus: None,
            keyboard_press: None,
            selected_entry: None,
            open_option: None,
            tooltip: ClassicTooltipTracker::new(),
            info_open: true,
            info_client_id: Some(client_id),
            info_only: true,
            info_static: false,
            info_close_label: String::new(),
            option_caption_reference: String::new(),
            option_caption_width: Cell::new(0),
            option_scroll_row: Cell::new(0),
            scroll_row: Cell::new(0),
        }
        .with_initial_info_lines(lines)
    }

    /// Presents the static-text `C4GUI::InfoDialog` constructor. Its input
    /// uses the native `|` line separator and has no one-second update hook.
    pub fn new_static_info(
        caption: impl Into<String>,
        requested_line_count: usize,
        text: &str,
        close_label: impl Into<String>,
    ) -> Self {
        Self {
            caption: String::new(),
            info_dialog: ScrollingInfoDialog::new(caption, requested_line_count, false),
            info_resources: RuntimeClientInfoResources::default(),
            caption_scroll: Cell::new(CaptionScrollState::default()),
            info_caption_scroll: Cell::new(CaptionScrollState::default()),
            options: Vec::new(),
            rows: Vec::new(),
            status: RuntimeClientListStatus::default(),
            dialog_offset: (0, 0),
            info_dialog_offset: (0, 0),
            pointer: None,
            pointer_capture: None,
            title_drag: None,
            focus: None,
            keyboard_press: None,
            selected_entry: None,
            open_option: None,
            tooltip: ClassicTooltipTracker::new(),
            info_open: true,
            info_client_id: None,
            info_only: true,
            info_static: true,
            info_close_label: close_label.into(),
            option_caption_reference: String::new(),
            option_caption_width: Cell::new(0),
            option_scroll_row: Cell::new(0),
            scroll_row: Cell::new(0),
        }
        .with_initial_info_lines(
            text.split('|')
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }

    fn with_initial_info_lines(mut self, lines: Vec<String>) -> Self {
        self.info_dialog.reset_lines(lines);
        self
    }

    pub fn with_info_caption(mut self, caption: impl Into<String>) -> Self {
        self.info_dialog.set_caption(caption);
        self
    }

    pub fn with_info_resources(mut self, resources: RuntimeClientInfoResources) -> Self {
        self.info_resources = resources;
        if let Some(client_id) = self.info_client_id {
            self.info_dialog.reset_lines(client_info_lines_for(
                &self.rows,
                client_id,
                &self.info_resources,
            ));
        }
        self
    }

    pub fn with_option_caption_reference(mut self, caption: impl Into<String>) -> Self {
        self.option_caption_reference = caption.into();
        self.option_caption_width.set(0);
        self
    }

    pub fn rows(&self) -> &[RuntimeClientRow] {
        &self.rows
    }

    pub fn option_rows(&self) -> &[LobbyOptionRow] {
        &self.options
    }

    pub const fn focused(&self) -> Option<RuntimeClientListFocus> {
        self.focus
    }

    pub const fn selected_client_id(&self) -> Option<i32> {
        match self.selected_entry {
            Some(
                RuntimeClientListSelection::Client(client_id)
                | RuntimeClientListSelection::Connection { client_id, .. },
            ) => Some(client_id),
            None => None,
        }
    }

    pub const fn selected_entry(&self) -> Option<RuntimeClientListSelection> {
        self.selected_entry
    }

    pub const fn open_option(&self) -> Option<LobbyOptionKind> {
        self.open_option
    }

    pub fn set_open_option(&mut self, option: Option<LobbyOptionKind>) {
        self.open_option = option.filter(|kind| {
            self.options
                .iter()
                .any(|row| row.kind == *kind && row.editable && !row.choices.is_empty())
        });
    }

    pub fn note_non_pointer_input(&mut self) {
        self.tooltip.note_non_pointer_input();
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

    pub const fn info_is_open(&self) -> bool {
        self.info_open || self.info_client_id.is_some()
    }

    pub fn info_lines(&self) -> &[String] {
        self.info_dialog.lines()
    }

    pub fn info_caption(&self) -> &str {
        self.info_dialog.caption()
    }

    pub const fn info_requested_line_count(&self) -> usize {
        self.info_dialog.requested_line_count()
    }

    pub const fn is_info_only(&self) -> bool {
        self.info_only
    }

    pub const fn is_static_info_only(&self) -> bool {
        self.info_only && self.info_static
    }

    pub const fn has_positional_pointer_drag(&self) -> bool {
        self.title_drag.is_some()
    }

    /// Whether CMouse's retained left-button drag element belongs to any
    /// client-list control, including ordinary buttons and scroll controls.
    pub const fn has_pointer_capture(&self) -> bool {
        self.title_drag.is_some() || self.pointer_capture.is_some()
    }

    /// Current first visible client/connection row for retained wheel state.
    pub fn scroll_row(&self, preferred: IntRect, font_line_height: i32) -> usize {
        let layout = self.layout(preferred, font_line_height);
        self.clamped_scroll_row(&layout)
    }

    pub fn replace_snapshot(
        &mut self,
        options: Vec<LobbyOptionRow>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
    ) {
        self.replace_snapshot_inner(options, rows, status, false);
    }

    pub fn replace_snapshot_on_sec1(
        &mut self,
        options: Vec<LobbyOptionRow>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
    ) {
        self.replace_snapshot_inner(options, rows, status, true);
    }

    fn replace_snapshot_inner(
        &mut self,
        options: Vec<LobbyOptionRow>,
        rows: Vec<RuntimeClientRow>,
        status: RuntimeClientListStatus,
        sec1_timer: bool,
    ) {
        if self.is_static_info_only() {
            return;
        }
        self.options = options;
        self.rows = rows;
        self.status = status;
        // A client that leaves while its info dialog is open does not close it:
        // the next UpdateText simply resolves nothing and prints the unknown-id
        // line (src/C4Network2Dialogs.cpp:54-59).
        if let Some(lines) = self
            .info_client_id
            .map(|id| client_info_lines_for(&self.rows, id, &self.info_resources))
        {
            if sec1_timer {
                let _ = self.info_dialog.on_sec1_timer(|| lines);
            } else {
                self.info_dialog.replace_lines_preserving_scroll(lines);
            }
        }
        if self
            .selected_entry
            .is_some_and(|selected| !self.contains_selection(selected))
        {
            self.selected_entry = None;
        }
        if self
            .focus
            .is_some_and(|focus| !self.focus_order().contains(&focus))
        {
            self.focus = Some(RuntimeClientListFocus::ClientList);
            self.keyboard_press = None;
        }
        self.set_open_option(self.open_option);
        self.scroll_row.set(
            self.scroll_row
                .get()
                .min(self.list_row_count().saturating_sub(1)),
        );
    }

    pub fn layout(&self, preferred: IntRect, font_line_height: i32) -> RuntimeClientListLayout {
        let (width, height) = if self.is_static_info_only() {
            (preferred.w.max(1), preferred.h.max(1))
        } else {
            (
                (preferred.w * 3 / 4).max(180).min(preferred.w.max(1)),
                (preferred.h * 3 / 4).max(120).min(preferred.h.max(1)),
            )
        };
        let bounds = IntRect::new(
            (preferred.x + (preferred.w - width) / 2).saturating_add(self.dialog_offset.0),
            (preferred.y + (preferred.h - height) / 2).saturating_add(self.dialog_offset.1),
            width,
            height,
        );
        let caption_height = (font_line_height + 8).max(24).min(height);
        let status_height = font_line_height
            .max(1)
            .min((height - caption_height).max(1));
        let client = IntRect::new(
            bounds.x + 4,
            bounds.y + caption_height + 3,
            (bounds.w - 8).max(1),
            (bounds.h - caption_height - status_height - 7).max(1),
        );
        let option_row_height = font_line_height.max(1).saturating_add(6);
        let option_count = i32::try_from(self.options.len()).unwrap_or(i32::MAX);
        let option_content_height = option_row_height
            .saturating_mul(option_count)
            .saturating_add(
                OPTION_ITEM_SPACING.saturating_mul(option_count.saturating_sub(1).max(0)),
            );
        let option_height = option_content_height
            .saturating_add(2 * LIST_BOX_MARGIN)
            .min(client.h / 2)
            .max(font_line_height.min(client.h));
        let options = IntRect::new(client.x, client.y, client.w, option_height);
        // ScrollWindow always reserves its 16-pixel scrollbar column. The
        // auto-scroll decoration only hides the bar when every item fits.
        let option_scrollbar = IntRect::new(
            options.x + options.w - LIST_BOX_MARGIN - SCROLLBAR_EXTENT,
            options.y + LIST_BOX_MARGIN,
            SCROLLBAR_EXTENT,
            (options.h - 2 * LIST_BOX_MARGIN).max(1),
        );
        let visible_option_rows = usize::try_from(
            (((options.h - 2 * LIST_BOX_MARGIN).max(1) + OPTION_ITEM_SPACING)
                / (option_row_height + OPTION_ITEM_SPACING).max(1))
            .max(1),
        )
        .unwrap_or(usize::MAX);
        let option_scroll_row = self
            .option_scroll_row
            .get()
            .min(self.options.len().saturating_sub(visible_option_rows));
        self.option_scroll_row.set(option_scroll_row);
        let option_rows = self
            .options
            .iter()
            .enumerate()
            .skip(option_scroll_row)
            .take(visible_option_rows)
            .enumerate()
            .map(|(visible_index, (index, _))| {
                let rect = IntRect::new(
                    options.x + LIST_BOX_MARGIN,
                    options.y
                        + LIST_BOX_MARGIN
                        + i32::try_from(visible_index)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(option_row_height + OPTION_ITEM_SPACING),
                    (options.w - 2 * LIST_BOX_MARGIN - SCROLLBAR_EXTENT).max(1),
                    option_row_height,
                );
                let caption_width = self
                    .option_caption_width(font_line_height)
                    .min((rect.w - 10).max(0));
                RuntimeClientOptionLayout {
                    index,
                    rect,
                    value: IntRect::new(
                        rect.x + caption_width + 8,
                        rect.y + 1,
                        (rect.w - caption_width - 9).max(1),
                        font_line_height.max(1) + 4,
                    ),
                }
            })
            .collect();
        let layout = RuntimeClientListLayout {
            bounds,
            caption: IntRect::new(bounds.x, bounds.y, bounds.w, caption_height),
            close_button: IntRect::new(
                bounds.x + bounds.w - 20,
                bounds.y + (caption_height - 16) / 2,
                16,
                16,
            ),
            options,
            option_scrollbar,
            list: IntRect::new(
                client.x,
                client.y + option_height + 2,
                client.w,
                (client.h - option_height - 2).max(1),
            ),
            status: IntRect::new(
                bounds.x + 4,
                bounds.y + bounds.h - status_height - 3,
                (bounds.w - 8).max(1),
                status_height,
            ),
            option_rows,
            row_height: (font_line_height + 4).max(18),
            icon_size: font_line_height.max(16),
            font_line_height: font_line_height.max(1),
        };
        self.clamped_scroll_row(&layout);
        layout
    }

    pub fn info_layout(
        &self,
        preferred: IntRect,
        font_line_height: i32,
    ) -> Option<RuntimeClientInfoLayout> {
        let parent = self.layout(preferred, font_line_height);
        self.info_layout_from_parent(&parent)
    }

    pub fn info_scroll_metrics(
        &self,
        preferred: IntRect,
        font: &ClonkFont,
    ) -> Option<ScrollingInfoMetrics> {
        let layout = self.info_layout(preferred, font.line_height)?;
        let geometry = info_scrolling_geometry(&layout, font.line_height);
        self.info_dialog
            .prepare_wrapped_lines(font, geometry.viewport.w);
        Some(self.info_dialog.metrics(&geometry))
    }

    /// Primes TextWindow wrapping for render-independent pointer, wheel and
    /// keyboard input. The cache uses interior mutability so app geometry
    /// queries can keep their read-only controller access.
    pub fn prepare_info_lines(&self, preferred: IntRect, font: &ClonkFont) {
        let Some(layout) = self.info_layout(preferred, font.line_height) else {
            return;
        };
        let geometry = info_scrolling_geometry(&layout, font.line_height);
        self.info_dialog
            .prepare_wrapped_lines(font, geometry.viewport.w);
    }

    pub fn visible_info_lines(&self, preferred: IntRect, font: &ClonkFont) -> Vec<String> {
        let Some(layout) = self.info_layout(preferred, font.line_height) else {
            return Vec::new();
        };
        let geometry = info_scrolling_geometry(&layout, font.line_height);
        self.info_dialog
            .prepare_wrapped_lines(font, geometry.viewport.w);
        self.info_dialog
            .visible_lines(&geometry)
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    /// Returns the title/close tooltip currently owned by this dialog. The
    /// caller supplies the pointer only after the process-global classic mouse
    /// tracker has reached its shared 500ms threshold.
    pub fn tooltip_at(
        &self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> Option<StartupTooltip> {
        let routed_pointer = self.pointer?;
        if routed_pointer.x as i32 != point.x as i32 || routed_pointer.y as i32 != point.y as i32 {
            return None;
        }

        let layout = self.layout(preferred, font_line_height);
        if let Some(info) = self.info_layout_from_parent(&layout) {
            if contains(info.close_button, point)
                || info
                    .bottom_close_button
                    .is_some_and(|button| contains(button, point))
            {
                Some(StartupTooltip::resource("IDS_MNU_CLOSE"))
            } else if contains(info.caption, point) {
                Some(StartupTooltip::text(self.info_dialog.caption().to_string()))
            } else {
                None
            }
        } else if contains(layout.close_button, point) {
            Some(StartupTooltip::resource("IDS_MNU_CLOSE"))
        } else if contains(layout.caption, point) && !self.caption.is_empty() {
            Some(StartupTooltip::text(self.caption.clone()))
        } else {
            None
        }
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
        self.tooltip.note_pointer_wheel();
        let layout = self.layout(preferred, font_line_height);
        if let Some(info) = self.info_layout_from_parent(&layout) {
            if contains(info.text_window, point) && delta != 0 {
                let geometry = info_scrolling_geometry(&info, font_line_height);
                self.info_dialog.handle_wheel(delta, &geometry);
            }
            return true;
        }
        if contains(layout.options, point) {
            if delta != 0 {
                let row_height = layout
                    .option_rows
                    .first()
                    .map(|row| row.rect.h.max(1) as usize)
                    .unwrap_or(1);
                let row_delta =
                    (delta.unsigned_abs() as usize).saturating_add(row_height - 1) / row_height;
                let current = self.option_scroll_row.get();
                let max_scroll = self.options.len().saturating_sub(layout.option_rows.len());
                let next = if delta > 0 {
                    current.saturating_sub(row_delta)
                } else {
                    current.saturating_add(row_delta).min(max_scroll)
                };
                self.option_scroll_row.set(next);
            }
            return true;
        }
        if !contains(layout.list, point) {
            return false;
        }
        if delta != 0 {
            let row_height = layout.row_height.max(1) as usize;
            let row_delta =
                (delta.unsigned_abs() as usize).saturating_add(row_height - 1) / row_height;
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
        self.handle_pointer_move_at(point, preferred, font_line_height, Instant::now())
    }

    pub fn handle_pointer_move_at(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
        now: Instant,
    ) -> bool {
        self.pointer = Some(point);
        self.tooltip.note_pointer_move_at(point, now);
        if self.update_title_drag(point) {
            return true;
        }
        let layout = self.layout(preferred, font_line_height);
        if self.pointer_capture == Some(HitTarget::InfoScrollTrack) {
            if let Some(info) = self.info_layout_from_parent(&layout) {
                let geometry = info_scrolling_geometry(&info, font_line_height);
                self.info_dialog.set_scroll_from_pointer(point, &geometry);
            }
            return true;
        }
        if self.pointer_capture == Some(HitTarget::OptionScrollTrack) {
            self.set_option_scroll_from_pointer(point, &layout);
            return true;
        }
        self.pointer_capture.is_some() || self.hit_target(point, &layout).is_some()
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> bool {
        self.pointer = Some(point);
        self.tooltip.note_pointer_button();
        self.keyboard_press = None;
        let layout = self.layout(preferred, font_line_height);
        if let Some(title) = self.title_at(point, &layout) {
            self.pointer_capture = None;
            self.title_drag = Some(TitleDrag {
                title,
                pointer: point,
                offset: self.title_offset(title),
            });
            return true;
        }
        self.title_drag = None;
        self.pointer_capture = self.hit_target(point, &layout);
        match self.pointer_capture {
            Some(HitTarget::InfoScrollUp) => {
                if let Some(info) = self.info_layout_from_parent(&layout) {
                    let geometry = info_scrolling_geometry(&info, font_line_height);
                    self.info_dialog
                        .activate_scroll_target(InfoScrollTarget::Up, point, &geometry);
                }
            }
            Some(HitTarget::InfoScrollDown) => {
                if let Some(info) = self.info_layout_from_parent(&layout) {
                    let geometry = info_scrolling_geometry(&info, font_line_height);
                    self.info_dialog.activate_scroll_target(
                        InfoScrollTarget::Down,
                        point,
                        &geometry,
                    );
                }
            }
            Some(HitTarget::InfoScrollTrack) => {
                if let Some(info) = self.info_layout_from_parent(&layout) {
                    let geometry = info_scrolling_geometry(&info, font_line_height);
                    self.info_dialog.activate_scroll_target(
                        InfoScrollTarget::Track,
                        point,
                        &geometry,
                    );
                }
            }
            Some(HitTarget::OptionRow(_) | HitTarget::OptionValue(_)) => {
                self.focus = Some(RuntimeClientListFocus::OptionsList);
            }
            Some(HitTarget::OptionScrollUp) => {
                self.focus = Some(RuntimeClientListFocus::OptionsList);
                self.scroll_options_by(-1, &layout);
            }
            Some(HitTarget::OptionScrollDown) => {
                self.focus = Some(RuntimeClientListFocus::OptionsList);
                self.scroll_options_by(1, &layout);
            }
            Some(HitTarget::OptionScrollTrack) => {
                self.focus = Some(RuntimeClientListFocus::OptionsList);
                self.set_option_scroll_from_pointer(point, &layout);
            }
            Some(HitTarget::ClientInfo(client_id)) => {
                self.focus = Some(RuntimeClientListFocus::ClientList);
                self.selected_entry = Some(RuntimeClientListSelection::Client(client_id));
            }
            Some(HitTarget::ConnectionRow(client_id, connection_id)) => {
                self.focus = Some(RuntimeClientListFocus::ClientList);
                self.selected_entry = Some(RuntimeClientListSelection::Connection {
                    client_id,
                    connection_id,
                });
            }
            Some(
                HitTarget::Mute(client_id)
                | HitTarget::Activate(client_id)
                | HitTarget::Kick(client_id),
            ) => {
                self.focus = Some(RuntimeClientListFocus::ClientList);
                self.selected_entry = Some(RuntimeClientListSelection::Client(client_id));
            }
            Some(HitTarget::Disconnect(client_id, connection_id)) => {
                self.focus = Some(RuntimeClientListFocus::ClientList);
                self.selected_entry = Some(RuntimeClientListSelection::Connection {
                    client_id,
                    connection_id,
                });
            }
            _ => {}
        }
        self.pointer_capture.is_some()
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        font_line_height: i32,
    ) -> Option<RuntimeClientListAction> {
        self.pointer = Some(point);
        self.tooltip.note_pointer_button();
        if self.update_title_drag(point) {
            self.title_drag = None;
            return None;
        }
        let pressed = self.pointer_capture.take()?;
        let layout = self.layout(preferred, font_line_height);
        let released = self.hit_target(point, &layout);
        if released != Some(pressed) {
            return None;
        }
        let action = match pressed {
            HitTarget::Close => RuntimeClientListAction::Close,
            HitTarget::InfoClose | HitTarget::InfoBottomClose => {
                self.close_info();
                RuntimeClientListAction::CloseInfo
            }
            HitTarget::OptionValue(index) => {
                let option = self.options.get(index)?;
                let value = layout
                    .option_rows
                    .iter()
                    .find(|row| row.index == index)?
                    .value;
                RuntimeClientListAction::OptionSelectionRequested {
                    option: option.kind,
                    anchor: GuiPoint::new(value.x as f32, (value.y + value.h) as f32),
                    minimum_width: value.w,
                }
            }
            HitTarget::ClientInfo(client_id) => {
                self.reset_info_presentation();
                self.info_dialog.reset_lines(client_info_lines_for(
                    &self.rows,
                    client_id,
                    &self.info_resources,
                ));
                self.info_open = true;
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
            HitTarget::Dialog
            | HitTarget::InfoScrollUp
            | HitTarget::InfoScrollDown
            | HitTarget::InfoScrollTrack
            | HitTarget::OptionRow(_)
            | HitTarget::OptionScrollUp
            | HitTarget::OptionScrollDown
            | HitTarget::OptionScrollTrack
            | HitTarget::ConnectionRow(_, _) => {
                return None;
            }
        };
        Some(action)
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pointer_capture = None;
        self.title_drag = None;
        self.tooltip.pointer_left();
    }

    /// Clear hover state when a screen-level popup occludes this dialog while
    /// preserving CMouse's retained drag element.
    pub fn pointer_occluded(&mut self) {
        self.pointer = None;
        self.tooltip.pointer_left();
    }

    pub fn handle_escape(&mut self, pressed: bool) -> Option<RuntimeClientListAction> {
        if !pressed {
            return None;
        }
        if self.info_is_open() {
            self.close_info();
            Some(RuntimeClientListAction::CloseInfo)
        } else {
            Some(RuntimeClientListAction::Close)
        }
    }

    fn info_layout_from_parent(
        &self,
        parent: &RuntimeClientListLayout,
    ) -> Option<RuntimeClientInfoLayout> {
        if !self.info_is_open() {
            return None;
        }
        let width = 620.min(parent.bounds.w).max(1);
        let font_line_height = parent.font_line_height;
        let height = self
            .info_dialog
            .preferred_dialog_height(font_line_height)
            .max(90)
            .min(parent.bounds.h);
        // The information dialog is a separate modal C4GUI::Dialog. Center it
        // in the preferred rectangle independently of a dragged F4 dialog.
        let parent_x = parent.bounds.x.saturating_sub(self.dialog_offset.0);
        let parent_y = parent.bounds.y.saturating_sub(self.dialog_offset.1);
        let bounds = IntRect::new(
            (parent_x + (parent.bounds.w - width) / 2).saturating_add(self.info_dialog_offset.0),
            (parent_y + (parent.bounds.h - height) / 2).saturating_add(self.info_dialog_offset.1),
            width,
            height,
        );
        let caption_height = if self.info_static {
            font_line_height.max(23)
        } else {
            (parent.row_height + 4).max(24)
        };
        let caption = IntRect::new(bounds.x, bounds.y, bounds.w, caption_height);
        let (text_window, bottom_close_button) = if self.info_static {
            let inner = IntRect::new(
                bounds.x + INFO_DIALOG_INDENT,
                caption.y + caption.h + INFO_DIALOG_INDENT,
                (bounds.w - 2 * INFO_DIALOG_INDENT).max(1),
                (bounds.h - caption.h - 2 * INFO_DIALOG_INDENT).max(1),
            );
            let button_area_height = INFO_BUTTON_AREA_HEIGHT.min(inner.h).max(1);
            let button_area = IntRect::new(
                inner.x,
                inner.y + inner.h - button_area_height,
                inner.w,
                button_area_height,
            );
            let close_width = INFO_CLOSE_BUTTON_WIDTH.min(button_area.w).max(1);
            (
                inner.with_height((inner.h - button_area_height - 2 * INFO_DIALOG_INDENT).max(1)),
                Some(IntRect::new(
                    button_area.x + (button_area.w - close_width) / 2,
                    button_area.y + (button_area.h - INFO_CLOSE_BUTTON_HEIGHT) / 2,
                    close_width,
                    INFO_CLOSE_BUTTON_HEIGHT.min(button_area.h).max(1),
                )),
            )
        } else {
            (
                IntRect::new(
                    bounds.x + 4,
                    caption.y + caption.h + 3,
                    (bounds.w - 8).max(1),
                    self.info_dialog
                        .preferred_text_window_height(font_line_height)
                        .min((bounds.h - caption.h - 7).max(1)),
                ),
                None,
            )
        };
        let scrolling = self.info_dialog.geometry(text_window, font_line_height);
        Some(RuntimeClientInfoLayout {
            bounds,
            caption,
            close_button: IntRect::new(
                bounds.x + bounds.w - 20,
                if self.info_static {
                    caption.y + 4
                } else {
                    caption.y + (caption.h - 16) / 2
                },
                16,
                16,
            ),
            bottom_close_button,
            text_window,
            text: scrolling.viewport,
            scrollbar: scrolling.scrollbar,
        })
    }

    fn title_at(&self, point: GuiPoint, layout: &RuntimeClientListLayout) -> Option<DialogTitle> {
        if let Some(info) = self.info_layout_from_parent(layout) {
            return (!contains(info.close_button, point) && contains(info.caption, point))
                .then_some(DialogTitle::Info);
        }
        (!self.info_only
            && !contains(layout.close_button, point)
            && contains(layout.caption, point))
        .then_some(DialogTitle::Main)
    }

    fn title_offset(&self, title: DialogTitle) -> (i32, i32) {
        match title {
            DialogTitle::Main => self.dialog_offset,
            DialogTitle::Info => self.info_dialog_offset,
        }
    }

    fn update_title_drag(&mut self, point: GuiPoint) -> bool {
        let Some(drag) = self.title_drag else {
            return false;
        };
        let offset = (
            drag.offset
                .0
                .saturating_add((point.x - drag.pointer.x) as i32),
            drag.offset
                .1
                .saturating_add((point.y - drag.pointer.y) as i32),
        );
        match drag.title {
            DialogTitle::Main => self.dialog_offset = offset,
            DialogTitle::Info => self.info_dialog_offset = offset,
        }
        true
    }

    fn reset_info_presentation(&mut self) {
        self.info_dialog_offset = (0, 0);
        self.info_caption_scroll.set(CaptionScrollState::default());
        self.info_dialog.reset_scroll();
        if self
            .title_drag
            .is_some_and(|drag| drag.title == DialogTitle::Info)
        {
            self.title_drag = None;
        }
    }

    fn close_info(&mut self) {
        self.info_open = false;
        self.info_client_id = None;
        self.pointer_capture = None;
        self.reset_info_presentation();
        self.info_dialog.reset_lines(Vec::new());
    }

    pub fn handle_key(
        &mut self,
        key: KeyCode,
        shift: bool,
        preferred: IntRect,
        font_line_height: i32,
    ) -> (bool, Option<RuntimeClientListAction>) {
        self.tooltip.note_non_pointer_input();
        if self.info_is_open() {
            self.keyboard_press = None;
            if key == KeyCode::Escape {
                return (true, self.handle_escape(true));
            }
            if self.info_static && key == KeyCode::Enter {
                self.close_info();
                return (true, Some(RuntimeClientListAction::CloseInfo));
            }
            let layout = self.layout(preferred, font_line_height);
            if let Some(info) = self.info_layout_from_parent(&layout) {
                let geometry = info_scrolling_geometry(&info, font_line_height);
                let _ = self.info_dialog.handle_key(key, &geometry);
            }
            return (true, None);
        }
        if !matches!(key, KeyCode::Enter | KeyCode::Space) {
            self.keyboard_press = None;
        }
        let layout = self.layout(preferred, font_line_height);
        match key {
            KeyCode::Escape => (true, self.handle_escape(true)),
            KeyCode::Tab => {
                self.advance_focus(shift, &layout);
                (true, None)
            }
            KeyCode::Up if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection(-1, &layout);
                (true, None)
            }
            KeyCode::Down if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection(1, &layout);
                (true, None)
            }
            KeyCode::Home if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection_to_edge(false, &layout);
                (true, None)
            }
            KeyCode::End if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection_to_edge(true, &layout);
                (true, None)
            }
            KeyCode::PageUp if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection_page(-1, &layout);
                (true, None)
            }
            KeyCode::PageDown if self.focus == Some(RuntimeClientListFocus::ClientList) => {
                self.move_list_selection_page(1, &layout);
                (true, None)
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
                if self.focus == Some(RuntimeClientListFocus::OptionsList) =>
            {
                // C4Network2ClientListDlg disables C4GameOptionsList row
                // selection. The shell owns list-navigation keys, but no
                // ComboBox child can acquire keyboard focus from it.
                (true, None)
            }
            KeyCode::Enter | KeyCode::Space => {
                let Some(focus) = self.focus else {
                    return (false, None);
                };
                if Self::activation_for_focus(focus).is_some() {
                    self.keyboard_press = Some((key, focus));
                    (true, None)
                } else {
                    self.keyboard_press = None;
                    (false, None)
                }
            }
            _ => (false, None),
        }
    }

    pub fn handle_key_release(&mut self, key: KeyCode) -> (bool, Option<RuntimeClientListAction>) {
        self.tooltip.note_non_pointer_input();
        let Some((pressed_key, pressed_focus)) = self.keyboard_press.take() else {
            return (false, None);
        };
        if pressed_key != key {
            return (false, None);
        }
        if self.focus != Some(pressed_focus) {
            return (true, None);
        }
        (true, Self::activation_for_focus(pressed_focus))
    }

    fn activation_for_focus(focus: RuntimeClientListFocus) -> Option<RuntimeClientListAction> {
        match focus {
            RuntimeClientListFocus::Close => Some(RuntimeClientListAction::Close),
            RuntimeClientListFocus::Mute(client_id) => {
                Some(RuntimeClientListAction::ToggleMute(client_id))
            }
            RuntimeClientListFocus::Activate(client_id) => {
                Some(RuntimeClientListAction::ToggleActivate(client_id))
            }
            RuntimeClientListFocus::Kick(client_id) => {
                Some(RuntimeClientListAction::Kick(client_id))
            }
            RuntimeClientListFocus::Disconnect {
                client_id,
                connection_id,
            } => Some(RuntimeClientListAction::Disconnect {
                client_id,
                connection_id,
            }),
            RuntimeClientListFocus::OptionsList | RuntimeClientListFocus::ClientList => None,
        }
    }

    fn advance_focus(&mut self, backwards: bool, layout: &RuntimeClientListLayout) {
        let order = self.focus_order();
        let next = match self
            .focus
            .and_then(|focus| order.iter().position(|candidate| *candidate == focus))
        {
            Some(current) if backwards => (current + order.len() - 1) % order.len(),
            Some(current) => (current + 1) % order.len(),
            None if backwards => order.len() - 1,
            None => 0,
        };
        self.focus = Some(order[next]);
        if self.focus == Some(RuntimeClientListFocus::ClientList) && self.selected_entry.is_none() {
            self.selected_entry = self
                .rows
                .first()
                .map(|row| RuntimeClientListSelection::Client(row.client_id));
        }
        self.ensure_selected_entry_visible(layout);
    }

    fn focus_order(&self) -> Vec<RuntimeClientListFocus> {
        let mut order = vec![
            RuntimeClientListFocus::Close,
            RuntimeClientListFocus::OptionsList,
            RuntimeClientListFocus::ClientList,
        ];
        if let Some(RuntimeClientListSelection::Client(client_id)) = self.selected_entry {
            let Some(row) = self.rows.iter().find(|row| row.client_id == client_id) else {
                return order;
            };
            if !row.local {
                order.push(RuntimeClientListFocus::Mute(row.client_id));
            }
            if !row.host && row.can_moderate {
                order.push(RuntimeClientListFocus::Activate(row.client_id));
                order.push(RuntimeClientListFocus::Kick(row.client_id));
            }
        } else if let Some(RuntimeClientListSelection::Connection {
            client_id,
            connection_id,
        }) = self.selected_entry
        {
            if self.rows.iter().any(|row| {
                row.client_id == client_id
                    && row.connections.iter().any(|connection| {
                        connection.connection_id == connection_id && connection.can_disconnect
                    })
            }) {
                order.push(RuntimeClientListFocus::Disconnect {
                    client_id,
                    connection_id,
                });
            }
        }
        order
    }

    fn list_selections(&self) -> Vec<RuntimeClientListSelection> {
        self.rows
            .iter()
            .flat_map(|row| {
                std::iter::once(RuntimeClientListSelection::Client(row.client_id)).chain(
                    row.connections.iter().map(|connection| {
                        RuntimeClientListSelection::Connection {
                            client_id: row.client_id,
                            connection_id: connection.connection_id,
                        }
                    }),
                )
            })
            .collect()
    }

    fn contains_selection(&self, selected: RuntimeClientListSelection) -> bool {
        self.list_selections().contains(&selected)
    }

    fn move_list_selection(&mut self, direction: i32, layout: &RuntimeClientListLayout) {
        let entries = self.list_selections();
        if entries.is_empty() {
            self.selected_entry = None;
            return;
        }
        let current = self
            .selected_entry
            .and_then(|selected| entries.iter().position(|entry| *entry == selected));
        let next = match current {
            Some(index) if direction < 0 => index.saturating_sub(1),
            Some(index) => (index + 1).min(entries.len() - 1),
            None if direction < 0 => entries.len() - 1,
            None => 0,
        };
        self.selected_entry = Some(entries[next]);
        self.ensure_selected_entry_visible(layout);
    }

    fn move_list_selection_to_edge(&mut self, end: bool, layout: &RuntimeClientListLayout) {
        let entries = self.list_selections();
        self.selected_entry = if end {
            entries.last().copied()
        } else {
            entries.first().copied()
        };
        self.ensure_selected_entry_visible(layout);
    }

    fn move_list_selection_page(&mut self, direction: i32, layout: &RuntimeClientListLayout) {
        let entries = self.list_selections();
        if entries.is_empty() {
            self.selected_entry = None;
            return;
        }
        let visible = Self::visible_list_row_count(layout).max(1);
        let scroll = self.clamped_scroll_row(layout);
        let current = self
            .selected_entry
            .and_then(|selected| entries.iter().position(|entry| *entry == selected))
            .unwrap_or(if direction < 0 { entries.len() - 1 } else { 0 });
        let next = if direction < 0 {
            if current > scroll {
                scroll
            } else {
                scroll.saturating_sub(visible)
            }
        } else {
            let last_visible = scroll
                .saturating_add(visible.saturating_sub(1))
                .min(entries.len() - 1);
            if current < last_visible {
                last_visible
            } else {
                scroll
                    .saturating_add(visible.saturating_mul(2).saturating_sub(1))
                    .min(entries.len() - 1)
            }
        };
        self.selected_entry = Some(entries[next]);
        self.ensure_selected_entry_visible(layout);
    }

    fn ensure_selected_entry_visible(&self, layout: &RuntimeClientListLayout) {
        let Some(selected) = self.selected_entry else {
            return;
        };
        let Some(selected_index) = self
            .list_selections()
            .iter()
            .position(|entry| *entry == selected)
        else {
            return;
        };
        let visible = Self::visible_list_row_count(layout).max(1);
        let current = self.clamped_scroll_row(layout);
        let next = if selected_index < current {
            selected_index
        } else if selected_index >= current.saturating_add(visible) {
            selected_index.saturating_add(1).saturating_sub(visible)
        } else {
            current
        };
        self.scroll_row.set(next.min(self.max_scroll_row(layout)));
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

    fn caption_scroll_offset_at(
        &self,
        now: Instant,
        font: &ClonkFont,
        caption: &IntRect,
        title: DialogTitle,
    ) -> i32 {
        let (text, state) = match title {
            DialogTitle::Main => (self.caption.as_str(), &self.caption_scroll),
            DialogTitle::Info => (self.info_dialog.caption(), &self.info_caption_scroll),
        };
        caption_scroll_offset_at(state, now, font, text, caption.w)
    }

    fn option_max_scroll(&self, layout: &RuntimeClientListLayout) -> usize {
        self.options.len().saturating_sub(layout.option_rows.len())
    }

    fn option_caption_width(&self, font_line_height: i32) -> i32 {
        let measured = self.option_caption_width.get();
        if measured > 0 {
            return measured;
        }
        let longest = if self.option_caption_reference.is_empty() {
            self.options
                .iter()
                .map(|option| crate::c4_presentation_text(&option.caption).chars().count())
                .max()
                .unwrap_or(0)
        } else {
            crate::c4_presentation_text(&self.option_caption_reference)
                .chars()
                .count()
        };
        i32::try_from(longest)
            .unwrap_or(i32::MAX)
            .saturating_mul((font_line_height.max(1) + 1) / 2)
            .saturating_mul(5)
            / 4
    }

    fn measure_option_caption_width(&self, font: &ClonkFont) {
        let reference = if self.option_caption_reference.is_empty() {
            self.options
                .iter()
                .max_by_key(|option| {
                    font.measure(&crate::c4_presentation_text(&option.caption), true)
                        .0
                })
                .map(|option| option.caption.as_str())
        } else {
            Some(self.option_caption_reference.as_str())
        };
        let width = reference.map_or(0, |caption| {
            font.measure(&crate::c4_presentation_text(caption), true)
                .0
                .saturating_mul(5)
                / 4
        });
        self.option_caption_width.set(width.max(0));
    }

    fn scroll_options_by(&self, rows: i32, layout: &RuntimeClientListLayout) {
        let current = self.option_scroll_row.get();
        let next = if rows < 0 {
            current.saturating_sub(rows.unsigned_abs() as usize)
        } else {
            current
                .saturating_add(rows as usize)
                .min(self.option_max_scroll(layout))
        };
        self.option_scroll_row.set(next);
    }

    fn set_option_scroll_from_pointer(&self, point: GuiPoint, layout: &RuntimeClientListLayout) {
        let max_scroll = self.option_max_scroll(layout);
        let max_pin = (layout.option_scrollbar.h - 3 * SCROLLBAR_EXTENT).max(0);
        if max_scroll == 0 || max_pin == 0 {
            return;
        }
        let pin = (point.y.floor() as i32
            - layout.option_scrollbar.y
            - SCROLLBAR_EXTENT
            - SCROLLBAR_EXTENT / 2)
            .clamp(0, max_pin);
        self.option_scroll_row
            .set(max_scroll.saturating_mul(pin as usize) / max_pin as usize);
    }

    fn option_scrollbar_pin(&self, layout: &RuntimeClientListLayout) -> i32 {
        let max_scroll = self.option_max_scroll(layout);
        let max_pin = (layout.option_scrollbar.h - 3 * SCROLLBAR_EXTENT).max(0);
        if max_scroll == 0 || max_pin == 0 {
            0
        } else {
            let scroll = self.option_scroll_row.get().min(max_scroll);
            (max_pin as usize).saturating_mul(scroll) as i32 / max_scroll as i32
        }
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_with_activity(surface, preferred, resources, active, active, gamma)
    }

    pub fn render_with_activity(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_at_with_activity(
            surface,
            preferred,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
            Instant::now(),
        )
    }

    /// Draw the dialog body while deferring its delayed tooltip to the
    /// screen-global pass that follows C4GUI::CMouse.
    pub fn render_body_with_activity(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_at_with_activity_and_tooltip(
            surface,
            preferred,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
            Instant::now(),
            false,
        )
    }

    pub fn render_static_info(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: StaticInfoDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_static_info_at(surface, preferred, resources, active, gamma, Instant::now())
    }

    fn render_static_info_at(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: StaticInfoDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        ensure!(
            self.is_static_info_only(),
            "static InfoDialog renderer requires static info state"
        );
        resources.validate()?;
        let parent = self.layout(preferred, resources.fonts.text.line_height);
        if let Some(info) = self.info_layout_from_parent(&parent) {
            self.draw_static_info(surface, &parent, &info, resources, active, gamma, now);
        }
        Ok(())
    }

    pub fn render_at(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        self.render_at_with_activity(surface, preferred, resources, active, active, gamma, now)
    }

    pub fn render_at_with_activity(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> Result<()> {
        self.render_at_with_activity_and_tooltip(
            surface,
            preferred,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
            now,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_at_with_activity_and_tooltip(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
        draw_tooltip: bool,
    ) -> Result<()> {
        if self.is_static_info_only() {
            return self.render_static_info_at(
                surface,
                preferred,
                StaticInfoDialogResources {
                    skin: resources.skin,
                    fonts: resources.fonts,
                    icons: resources.icons,
                    button_highlight: resources.button_highlight,
                    scroll: resources.scroll,
                },
                mouse_active,
                gamma,
                now,
            );
        }
        resources.validate()?;
        self.measure_option_caption_width(&resources.fonts.text);
        let layout = self.layout(preferred, resources.fonts.text.line_height);
        if self.info_only {
            if let Some(info) = self.info_layout_from_parent(&layout) {
                self.draw_client_info(
                    surface,
                    &layout,
                    &info,
                    resources,
                    keyboard_active,
                    mouse_active,
                    gamma,
                    now,
                );
            }
            return Ok(());
        }
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        let caption_scroll = self.caption_scroll_offset_at(
            now,
            &resources.fonts.text,
            &layout.caption,
            DialogTitle::Main,
        );
        resources.skin.draw_caption_scrolled(
            surface,
            layout.caption,
            &self.caption,
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            TITLE_RIGHT_INDENT,
            caption_scroll,
            gamma,
        );
        self.draw_icon_button(
            surface,
            layout.close_button,
            ICON_CLOSE,
            HitTarget::Close,
            &layout,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
        );

        for row_layout in &layout.option_rows {
            if row_layout.rect.y + row_layout.rect.h > layout.options.y + layout.options.h {
                break;
            }
            let Some(option) = self.options.get(row_layout.index) else {
                continue;
            };
            let caption = format!("{}:", crate::c4_presentation_text(&option.caption));
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row_layout.rect.x + 1,
                row_layout.rect.y + (row_layout.rect.h - resources.fonts.text.line_height) / 2,
                &caption,
                [255, 255, 255, 255],
                TextAlign::Left,
                gamma,
                IntRect::new(
                    row_layout.rect.x + 1,
                    row_layout.rect.y,
                    (row_layout.value.x - row_layout.rect.x - 8).max(1),
                    row_layout.rect.h,
                ),
            );
            let arrow_x = row_layout.value.x + row_layout.value.w - CONTEXT_HEIGHT as i32 - 1;
            if option.editable {
                draw_engine_box(
                    surface,
                    row_layout.value.x,
                    row_layout.value.y,
                    row_layout.value.x + row_layout.value.w - 1,
                    row_layout.value.y + row_layout.value.h - 1,
                    STANDARD_BACKGROUND_COLOR,
                    gamma,
                );
                draw_3d_frame(surface, row_layout.value, gamma);
                draw_facet_stretch(
                    surface,
                    resources.context,
                    (
                        (u32::from(self.open_option == Some(option.kind)) * CONTEXT_HEIGHT) as f32,
                        0.0,
                        CONTEXT_HEIGHT as f32,
                        CONTEXT_HEIGHT as f32,
                    ),
                    (
                        arrow_x as f32,
                        (row_layout.value.y + (row_layout.value.h - CONTEXT_HEIGHT as i32) / 2)
                            as f32,
                        CONTEXT_HEIGHT as f32,
                        CONTEXT_HEIGHT as f32,
                    ),
                    gamma,
                );
            }
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                row_layout.value.x + CONTEXT_HEIGHT as i32 + 2,
                row_layout.value.y + (row_layout.value.h - resources.fonts.text.line_height) / 2,
                &crate::c4_presentation_text(&option.value),
                [255, 255, 255, 255],
                TextAlign::Left,
                gamma,
                IntRect::new(
                    row_layout.value.x,
                    row_layout.value.y,
                    (arrow_x - row_layout.value.x).max(1),
                    row_layout.value.h,
                ),
            );
            let hovered = mouse_active
                && self.pointer.is_some_and(|point| {
                    self.hit_target(point, &layout)
                        == Some(HitTarget::OptionValue(row_layout.index))
                });
            if option.editable && (hovered || self.open_option == Some(option.kind)) {
                draw_highlight(surface, row_layout.value, resources.button_highlight, gamma);
            }
        }
        let option_max_scroll = self.option_max_scroll(&layout);
        if option_max_scroll > 0 {
            draw_scrollbar(
                surface,
                layout.option_scrollbar,
                resources.scroll,
                self.option_scrollbar_pin(&layout),
                option_max_scroll,
                mouse_active && self.pointer_capture == Some(HitTarget::OptionScrollUp),
                mouse_active && self.pointer_capture == Some(HitTarget::OptionScrollDown),
                gamma,
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
        self.draw_rows(
            surface,
            &layout,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
        );
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

        if let Some(info) = self.info_layout_from_parent(&layout) {
            self.draw_client_info(
                surface,
                &layout,
                &info,
                resources,
                keyboard_active,
                mouse_active,
                gamma,
                now,
            );
        } else if mouse_active && draw_tooltip {
            if let Some(tooltip) = self.tooltip_state_at(now, preferred, &resources.fonts.text) {
                draw_classic_tooltip(
                    surface,
                    resources.tooltip_font,
                    tooltip.pointer,
                    &tooltip.text,
                    gamma,
                );
            }
        }
        Ok(())
    }

    /// Draw only the delayed list/option tooltip in the host's final overlay
    /// pass. Title and close tooltips remain owned by the global dialog pass.
    pub fn render_tooltip(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: RuntimeClientListResources<'_>,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) -> Result<bool> {
        resources.validate()?;
        self.measure_option_caption_width(&resources.fonts.text);
        let layout = self.layout(preferred, resources.fonts.text.line_height);
        if !mouse_active || self.info_layout_from_parent(&layout).is_some() {
            return Ok(false);
        }
        let Some(tooltip) = self.tooltip_state_at(Instant::now(), preferred, &resources.fonts.text)
        else {
            return Ok(false);
        };
        draw_classic_tooltip(
            surface,
            resources.tooltip_font,
            tooltip.pointer,
            &tooltip.text,
            gamma,
        );
        Ok(true)
    }

    fn draw_rows(
        &self,
        surface: &mut Surface,
        layout: &RuntimeClientListLayout,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
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
                    if self.selected_entry
                        == Some(RuntimeClientListSelection::Client(row.client_id))
                    {
                        draw_engine_box(
                            surface,
                            layout.list.x + 2,
                            y,
                            layout.list.x + layout.list.w - 3,
                            y + layout.row_height - 1,
                            if keyboard_active
                                && self.focus == Some(RuntimeClientListFocus::ClientList)
                            {
                                LIST_SELECTION
                            } else {
                                LIST_SELECTION_INACTIVE
                            },
                            gamma,
                        );
                    }
                    let status_rect = IntRect::new(
                        layout.list.x + 3,
                        y + (layout.row_height - layout.icon_size) / 2,
                        layout.icon_size,
                        layout.icon_size,
                    );
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
                                IntRect::new(
                                    right,
                                    status_rect.y,
                                    layout.icon_size,
                                    layout.icon_size,
                                ),
                                phase,
                                target,
                                layout,
                                resources,
                                keyboard_active,
                                mouse_active,
                                gamma,
                            );
                            right -= 2;
                        }
                    }
                    if !row.local {
                        right -= layout.icon_size;
                        self.draw_icon_button(
                            surface,
                            IntRect::new(right, status_rect.y, layout.icon_size, layout.icon_size),
                            if row.muted { ICON_NO_SOUND } else { ICON_SOUND },
                            HitTarget::Mute(row.client_id),
                            layout,
                            resources,
                            keyboard_active,
                            mouse_active,
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
                            IntRect::new(right, y, 54, layout.row_height),
                        );
                    }
                    let label_rect = client_label_rect(row, layout, y);
                    draw_clipped_text(
                        surface,
                        &resources.fonts.text,
                        label_rect.x,
                        y + 2,
                        &row.label(),
                        [255, 255, 255, 255],
                        TextAlign::Left,
                        gamma,
                        label_rect,
                    );
                }
                RuntimeListEntry::Connection {
                    client_id,
                    connection,
                } => {
                    if self.selected_entry
                        == Some(RuntimeClientListSelection::Connection {
                            client_id,
                            connection_id: connection.connection_id,
                        })
                    {
                        draw_engine_box(
                            surface,
                            layout.list.x + 2,
                            y,
                            layout.list.x + layout.list.w - 3,
                            y + layout.row_height - 1,
                            if keyboard_active
                                && self.focus == Some(RuntimeClientListFocus::ClientList)
                            {
                                LIST_SELECTION
                            } else {
                                LIST_SELECTION_INACTIVE
                            },
                            gamma,
                        );
                    }
                    let mut connection_right = layout.list.x + layout.list.w - 3;
                    if connection.can_disconnect {
                        connection_right -= layout.icon_size;
                        self.draw_icon_button(
                            surface,
                            IntRect::new(
                                connection_right,
                                y + (layout.row_height - layout.icon_size) / 2,
                                layout.icon_size,
                                layout.icon_size,
                            ),
                            ICON_DISCONNECT,
                            HitTarget::Disconnect(client_id, connection.connection_id),
                            layout,
                            resources,
                            keyboard_active,
                            mouse_active,
                            gamma,
                        );
                        connection_right -= 2;
                    }
                    // ConnectionListItem::Update shows getLag()
                    // (src/C4Network2Dialogs.cpp:357-369).
                    let ping = if connection.lag_ms < 0 {
                        "???".to_string()
                    } else {
                        format!("{} ms", connection.lag_ms)
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
                        IntRect::new(connection_right, y, 54, layout.row_height),
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
                        IntRect::new(
                            layout.list.x + layout.icon_size * 2,
                            y,
                            (connection_right - layout.list.x - layout.icon_size * 2).max(1),
                            layout.row_height,
                        ),
                    );
                }
            }
            y += layout.row_height;
        }
    }

    fn draw_static_info(
        &self,
        surface: &mut Surface,
        parent: &RuntimeClientListLayout,
        layout: &RuntimeClientInfoLayout,
        resources: StaticInfoDialogResources<'_>,
        active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) {
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        let geometry = info_scrolling_geometry(layout, resources.fonts.text.line_height);
        self.info_dialog
            .prepare_wrapped_lines(&resources.fonts.text, geometry.viewport.w);
        let caption_scroll = self.caption_scroll_offset_at(
            now,
            &resources.fonts.text,
            &layout.caption,
            DialogTitle::Info,
        );
        resources.skin.draw_caption_scrolled(
            surface,
            layout.caption,
            self.info_dialog.caption(),
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            TITLE_RIGHT_INDENT,
            caption_scroll,
            gamma,
        );

        let pointer_target = self
            .pointer
            .and_then(|point| self.hit_target(point, parent));
        let close_hovered = active && pointer_target == Some(HitTarget::InfoClose);
        let close_pressed = close_hovered && self.pointer_capture == Some(HitTarget::InfoClose);
        if close_hovered && !close_pressed {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }
        draw_icon(
            surface,
            layout.close_button,
            resources.icons,
            ICON_CLOSE,
            gamma,
        );
        if close_pressed {
            draw_highlight(
                surface,
                layout.close_button,
                resources.button_highlight,
                gamma,
            );
        }

        draw_engine_box(
            surface,
            layout.text_window.x,
            layout.text_window.y,
            layout.text_window.x + layout.text_window.w - 1,
            layout.text_window.y + layout.text_window.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.text_window, gamma);
        for line in self.info_dialog.visible_lines(&geometry) {
            draw_clipped_text_with_markup(
                surface,
                &resources.fonts.text,
                geometry.viewport.x,
                line.y,
                &line.text,
                [255, 255, 255, 255],
                TextAlign::Left,
                gamma,
                geometry.viewport,
                false,
            );
        }
        let metrics = self.info_dialog.metrics(&geometry);
        draw_scrollbar(
            surface,
            geometry.scrollbar,
            resources.scroll,
            self.info_dialog.scrollbar_pin(&geometry),
            usize::try_from(metrics.max_scroll).unwrap_or(usize::MAX),
            self.pointer_capture == Some(HitTarget::InfoScrollUp),
            self.pointer_capture == Some(HitTarget::InfoScrollDown),
            gamma,
        );

        if let Some(button) = layout.bottom_close_button {
            let hovered = active && pointer_target == Some(HitTarget::InfoBottomClose);
            resources.skin.draw_button(
                surface,
                button,
                &self.info_close_label,
                resources.fonts,
                ClassicButtonState {
                    pressed: hovered && self.pointer_capture == Some(HitTarget::InfoBottomClose),
                    highlighted: hovered,
                },
                gamma,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_client_info(
        &self,
        surface: &mut Surface,
        parent: &RuntimeClientListLayout,
        layout: &RuntimeClientInfoLayout,
        resources: RuntimeClientListResources<'_>,
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) {
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        let geometry = info_scrolling_geometry(layout, resources.fonts.text.line_height);
        self.info_dialog
            .prepare_wrapped_lines(&resources.fonts.text, geometry.viewport.w);
        let caption_scroll = self.caption_scroll_offset_at(
            now,
            &resources.fonts.text,
            &layout.caption,
            DialogTitle::Info,
        );
        resources.skin.draw_caption_scrolled(
            surface,
            layout.caption,
            self.info_dialog.caption(),
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            TITLE_RIGHT_INDENT,
            caption_scroll,
            gamma,
        );
        self.draw_icon_button(
            surface,
            layout.close_button,
            ICON_CLOSE,
            HitTarget::InfoClose,
            parent,
            resources,
            keyboard_active,
            mouse_active,
            gamma,
        );
        draw_engine_box(
            surface,
            layout.text_window.x,
            layout.text_window.y,
            layout.text_window.x + layout.text_window.w - 1,
            layout.text_window.y + layout.text_window.h - 1,
            0x7f00_0000,
            gamma,
        );
        draw_3d_frame(surface, layout.text_window, gamma);
        for line in self.info_dialog.visible_lines(&geometry) {
            draw_clipped_text(
                surface,
                &resources.fonts.text,
                geometry.viewport.x,
                line.y,
                &line.text,
                [255, 255, 255, 255],
                TextAlign::Left,
                gamma,
                geometry.viewport,
            );
        }
        let metrics = self.info_dialog.metrics(&geometry);
        draw_scrollbar(
            surface,
            geometry.scrollbar,
            resources.scroll,
            self.info_dialog.scrollbar_pin(&geometry),
            usize::try_from(metrics.max_scroll).unwrap_or(usize::MAX),
            self.pointer_capture == Some(HitTarget::InfoScrollUp),
            self.pointer_capture == Some(HitTarget::InfoScrollDown),
            gamma,
        );
    }

    pub fn tooltip_state_at(
        &self,
        now: Instant,
        preferred: IntRect,
        font: &ClonkFont,
    ) -> Option<RuntimeClientListTooltip> {
        let pointer = self.tooltip.eligible_pointer_at(now)?;
        self.measure_option_caption_width(font);
        let layout = self.layout(preferred, font.line_height);
        for row_layout in &layout.option_rows {
            if row_layout.rect.y + row_layout.rect.h > layout.options.y + layout.options.h {
                break;
            }
            if contains(row_layout.rect, pointer) {
                let text = self.options.get(row_layout.index)?.tooltip.clone();
                return (!text.is_empty()).then_some(RuntimeClientListTooltip { pointer, text });
            }
        }
        let mut y = layout.list.y + 2;
        let scroll_row = self.clamped_scroll_row(&layout);
        for entry in self.list_entries().skip(scroll_row) {
            if y + layout.row_height > layout.list.y + layout.list.h {
                break;
            }
            if let RuntimeListEntry::Client(row) = entry {
                let mut label_rect = client_label_rect(row, &layout, y);
                label_rect.y += 2;
                label_rect.h = font.line_height;
                label_rect.w = label_rect.w.min(font.measure(&row.label(), true).0.max(0));
                if contains(label_rect, pointer) && !row.player_names.is_empty() {
                    return Some(RuntimeClientListTooltip {
                        pointer,
                        text: row.player_names.join(", "),
                    });
                }
            }
            y += layout.row_height;
        }
        None
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
        keyboard_active: bool,
        mouse_active: bool,
        gamma: Option<&GammaRamp>,
    ) {
        let (highlighted, pressed) =
            self.icon_button_state(target, layout, keyboard_active, mouse_active);
        if highlighted && !pressed {
            draw_highlight(surface, rect, resources.button_highlight, gamma);
        }
        draw_icon(surface, rect, resources.icons, phase, gamma);
        if pressed {
            draw_highlight(surface, rect, resources.button_highlight, gamma);
        }
    }

    fn icon_button_state(
        &self,
        target: HitTarget,
        layout: &RuntimeClientListLayout,
        keyboard_active: bool,
        mouse_active: bool,
    ) -> (bool, bool) {
        let keyboard_focused = match target {
            HitTarget::Close => self.focus == Some(RuntimeClientListFocus::Close),
            HitTarget::Mute(client_id) => {
                self.focus == Some(RuntimeClientListFocus::Mute(client_id))
            }
            HitTarget::Activate(client_id) => {
                self.focus == Some(RuntimeClientListFocus::Activate(client_id))
            }
            HitTarget::Kick(client_id) => {
                self.focus == Some(RuntimeClientListFocus::Kick(client_id))
            }
            HitTarget::Disconnect(client_id, connection_id) => {
                self.focus
                    == Some(RuntimeClientListFocus::Disconnect {
                        client_id,
                        connection_id,
                    })
            }
            _ => false,
        };
        let keyboard_highlighted = keyboard_active && keyboard_focused;
        let pointer_highlighted = mouse_active
            && self.pointer.is_some_and(|point| {
                self.hit_target(point, layout)
                    .is_some_and(|hit| hit == target)
            });
        let keyboard_pressed = keyboard_highlighted
            && self
                .keyboard_press
                .is_some_and(|(_, focus)| self.focus == Some(focus));
        let pointer_pressed = pointer_highlighted && self.pointer_capture == Some(target);
        (
            keyboard_highlighted || pointer_highlighted,
            keyboard_pressed || pointer_pressed,
        )
    }

    fn hit_target(&self, point: GuiPoint, layout: &RuntimeClientListLayout) -> Option<HitTarget> {
        if let Some(info) = self.info_layout_from_parent(layout) {
            return if contains(info.close_button, point) {
                Some(HitTarget::InfoClose)
            } else if info
                .bottom_close_button
                .is_some_and(|button| contains(button, point))
            {
                Some(HitTarget::InfoBottomClose)
            } else if let Some(target) = self.info_dialog.scroll_target_at(
                point,
                &info_scrolling_geometry(&info, layout.font_line_height),
            ) {
                Some(match target {
                    InfoScrollTarget::Up => HitTarget::InfoScrollUp,
                    InfoScrollTarget::Down => HitTarget::InfoScrollDown,
                    InfoScrollTarget::Track => HitTarget::InfoScrollTrack,
                })
            } else if contains(info.bounds, point) {
                Some(HitTarget::Dialog)
            } else {
                None
            };
        }
        if contains(layout.close_button, point) {
            return Some(HitTarget::Close);
        }
        if self.option_max_scroll(layout) > 0 && contains(layout.option_scrollbar, point) {
            if point.y < (layout.option_scrollbar.y + SCROLLBAR_EXTENT) as f32 {
                return Some(HitTarget::OptionScrollUp);
            }
            if point.y
                >= (layout.option_scrollbar.y + layout.option_scrollbar.h - SCROLLBAR_EXTENT) as f32
            {
                return Some(HitTarget::OptionScrollDown);
            }
            return Some(HitTarget::OptionScrollTrack);
        }
        for row_layout in &layout.option_rows {
            if row_layout.rect.y + row_layout.rect.h > layout.options.y + layout.options.h {
                break;
            }
            let Some(option) = self.options.get(row_layout.index) else {
                continue;
            };
            if option.editable && contains(row_layout.value, point) {
                return Some(HitTarget::OptionValue(row_layout.index));
            }
            if contains(row_layout.rect, point) {
                return Some(HitTarget::OptionRow(row_layout.index));
            }
        }
        let mut y = layout.list.y + 2;
        let scroll_row = self.clamped_scroll_row(layout);
        for entry in self.list_entries().skip(scroll_row) {
            if y + layout.row_height > layout.list.y + layout.list.h {
                break;
            }
            match entry {
                RuntimeListEntry::Client(row) => {
                    let row_rect = IntRect::new(
                        layout.list.x + 2,
                        y,
                        (layout.list.w - 4).max(1),
                        layout.row_height,
                    );
                    let mut right = layout.list.x + layout.list.w - 3;
                    if !row.host && row.can_moderate {
                        right -= layout.icon_size;
                        let kick = IntRect::new(
                            right,
                            y + (layout.row_height - layout.icon_size) / 2,
                            layout.icon_size,
                            layout.icon_size,
                        );
                        if contains(kick, point) {
                            return Some(HitTarget::Kick(row.client_id));
                        }
                        right -= layout.icon_size + 2;
                        let activate = kick.with_x(right);
                        if contains(activate, point) {
                            return Some(HitTarget::Activate(row.client_id));
                        }
                        right -= 2;
                    }
                    if !row.local {
                        right -= layout.icon_size;
                        let mute = IntRect::new(
                            right,
                            y + (layout.row_height - layout.icon_size) / 2,
                            layout.icon_size,
                            layout.icon_size,
                        );
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
                } => {
                    if connection.can_disconnect {
                        let disconnect = IntRect::new(
                            layout.list.x + layout.list.w - 3 - layout.icon_size,
                            y + (layout.row_height - layout.icon_size) / 2,
                            layout.icon_size,
                            layout.icon_size,
                        );
                        if contains(disconnect, point) {
                            return Some(HitTarget::Disconnect(
                                client_id,
                                connection.connection_id,
                            ));
                        }
                    }
                    let row_rect = IntRect::new(
                        layout.list.x + 2,
                        y,
                        (layout.list.w - 4).max(1),
                        layout.row_height,
                    );
                    if contains(row_rect, point) {
                        return Some(HitTarget::ConnectionRow(
                            client_id,
                            connection.connection_id,
                        ));
                    }
                }
            }
            y += layout.row_height;
        }
        contains(layout.bounds, point).then_some(HitTarget::Dialog)
    }
}

/// `C4Network2ClientDlg::UpdateText` binds to a client *id*, not to a row, and
/// looks the client up on every update. An id that is not in the current list —
/// because the context entry went stale, or because the client left while the
/// dialog is open — renders the native fallback line rather than closing
/// (src/C4Network2Dialogs.cpp:54-59).
fn client_info_lines_for(
    rows: &[RuntimeClientRow],
    client_id: i32,
    resources: &RuntimeClientInfoResources,
) -> Vec<String> {
    rows.iter()
        .find(|row| row.client_id == client_id)
        .map_or_else(
            || {
                let client_id = client_id.to_string();
                vec![format_resource_string(&resources.unknown_id, &[&client_id])]
            },
            |row| client_info_lines(row, resources),
        )
}

fn client_info_lines(
    row: &RuntimeClientRow,
    resources: &RuntimeClientInfoResources,
) -> Vec<String> {
    let role = if row.host {
        &resources.host
    } else {
        &resources.client
    };
    let location = if row.local {
        &resources.local
    } else {
        &resources.remote
    };
    let activity = if row.activated {
        &resources.active
    } else {
        &resources.inactive
    };
    // The native format string ends in the acknowledgement marker
    // (`IDS_NET_CLIENT_INFO_FORMAT` = `%s %s %s %s (ID #%d):%s`).
    let acknowledgement = if row.unacknowledged { " (!ack)" } else { "" };
    // C4Network2ClientDlg::UpdateText passes C4Client::getName(), not the
    // runtime list's name:nick presentation label.
    let name = &row.name;
    let client_id = row.client_id.to_string();
    let mut lines = vec![format_resource_string(
        &resources.format,
        &[activity, location, role, name, &client_id, acknowledgement],
    )];
    if row.addresses.is_empty() {
        lines.push(resources.noaddresses.clone());
    } else {
        lines.push(resources.addresses.clone());
        lines.extend(
            row.addresses
                .iter()
                .enumerate()
                .map(|(index, address)| format!("  {index}: {address}")),
        );
    }
    if row.connections.is_empty() {
        lines.push(resources.noconnections.clone());
    } else {
        let mut connections = row.connections.iter().collect::<Vec<_>>();
        // C4Network2ClientDlg::UpdateText emits the message route first and
        // the separate data route second, regardless of container order
        // (src/C4Network2Dialogs.cpp:91-102).
        connections.sort_by_key(|connection| {
            (
                match connection.usage.as_str() {
                    "Data/Msg" | "Msg" => 0,
                    "Data" => 1,
                    _ => 2,
                },
                connection.connection_id,
            )
        });
        lines.extend(connections.into_iter().map(|connection| {
            // The info text shows getPingTime(), unlike the list rows
            // (src/C4Network2Dialogs.cpp:92-102).
            let ping_ms = connection.ping_ms.to_string();
            let usage = if connection.usage == "Data/Msg" {
                "Msg/Data"
            } else {
                connection.usage.as_str()
            };
            let template = if connection.usage == "Data" {
                &resources.conndata
            } else {
                &resources.connections
            };
            if connection.usage == "Data" {
                format_resource_string(
                    template,
                    &[&connection.protocol, &connection.peer_address, &ping_ms],
                )
            } else {
                format_resource_string(
                    template,
                    &[
                        usage,
                        &connection.protocol,
                        &connection.peer_address,
                        &ping_ms,
                    ],
                )
            }
        }));
    }
    lines
}

fn format_resource_string(template: &str, arguments: &[&str]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remainder = template;
    for argument in arguments {
        let placeholder = [
            remainder.find("%s"),
            remainder.find("%d"),
            remainder.find("%i"),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(placeholder) = placeholder else {
            break;
        };
        output.push_str(&remainder[..placeholder]);
        output.push_str(argument);
        remainder = &remainder[placeholder + 2..];
    }
    output.push_str(remainder);
    output
}

fn info_scrolling_geometry(
    layout: &RuntimeClientInfoLayout,
    line_height: i32,
) -> ScrollingInfoGeometry {
    ScrollingInfoGeometry {
        frame: layout.text_window,
        viewport: layout.text,
        scrollbar: layout.scrollbar,
        line_height: line_height.max(1),
    }
}

fn caption_scroll_offset_at(
    state: &Cell<CaptionScrollState>,
    now: Instant,
    font: &ClonkFont,
    text: &str,
    caption_width: i32,
) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let max_scroll = (font.measure(text, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT
        - caption_width)
        .max(0);
    advance_caption_scroll(state, now, max_scroll, TITLE_SCROLL_DELAY)
}

fn client_label_rect(row: &RuntimeClientRow, layout: &RuntimeClientListLayout, y: i32) -> IntRect {
    let status_x = layout.list.x + 3;
    let mut right = layout.list.x + layout.list.w - 3;
    if !row.host && row.can_moderate {
        right -= layout.icon_size * 2 + 4;
    }
    if !row.local {
        right -= layout.icon_size + 2;
    }
    if row.wait_ms.is_some() {
        right -= 54;
    }
    let x = status_x + layout.icon_size + 3;
    IntRect::new(x, y, (right - x - 2).max(1), layout.row_height)
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

#[allow(clippy::too_many_arguments)]
fn draw_scrollbar(
    surface: &mut Surface,
    rect: IntRect,
    scroll: &ImageData,
    pin: i32,
    max_scroll: usize,
    top_down: bool,
    bottom_down: bool,
    gamma: Option<&GammaRamp>,
) {
    if rect.h <= 0 {
        return;
    }
    let top_x = if top_down { 16.0 } else { 0.0 };
    let bottom_x = if bottom_down { 16.0 } else { 0.0 };
    draw_facet_stretch(
        surface,
        scroll,
        (top_x, 0.0, 16.0, 16.0),
        (rect.x as f32, rect.y as f32, 16.0, 16.0),
        gamma,
    );
    let mut y = SCROLLBAR_EXTENT;
    while y < rect.h - 5 {
        let height = SCROLLBAR_EXTENT.min(rect.h - 5 - y);
        if height <= 0 {
            break;
        }
        draw_facet_stretch(
            surface,
            scroll,
            (0.0, 16.0, 16.0, height as f32),
            (
                rect.x as f32,
                (rect.y + y) as f32,
                SCROLLBAR_EXTENT as f32,
                height as f32,
            ),
            gamma,
        );
        y += SCROLLBAR_EXTENT;
    }
    draw_facet_stretch(
        surface,
        scroll,
        (bottom_x, 32.0, 16.0, 16.0),
        (
            rect.x as f32,
            (rect.y + rect.h - SCROLLBAR_EXTENT) as f32,
            SCROLLBAR_EXTENT as f32,
            SCROLLBAR_EXTENT as f32,
        ),
        gamma,
    );
    if max_scroll > 0 && rect.h > 3 * SCROLLBAR_EXTENT {
        draw_facet_stretch(
            surface,
            scroll,
            (16.0, 16.0, 16.0, 16.0),
            (
                rect.x as f32,
                (rect.y + SCROLLBAR_EXTENT + pin) as f32,
                SCROLLBAR_EXTENT as f32,
                SCROLLBAR_EXTENT as f32,
            ),
            gamma,
        );
    }
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(
        surface,
        &clonk_gui::Rect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
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
    use crate::context_menu::{ClassicTooltipTracker, CLASSIC_TOOLTIP_DELAY};
    use crate::game_lobby::{core_runtime_option_rows, LobbyOptionLabels};
    use clonk_graphics::Color;

    fn unit_width_font(characters: &str) -> ClonkFont {
        let mut font = ClonkFont::new(3);
        font.h_space = 0;
        for character in characters.chars() {
            font.add_glyph(
                character,
                clonk_graphics::clonk_font::GlyphCell {
                    width: 1,
                    pixels: vec![Color::opaque(255, 255, 255); 4],
                },
            );
        }
        font
    }

    fn options(host: bool) -> Vec<LobbyOptionRow> {
        core_runtime_option_rows(host, host, false, &LobbyOptionLabels::default(), 1, 4, true)
    }

    fn tooltip_font() -> ClonkFont {
        let mut font = ClonkFont::new(16);
        for byte in b' '..=b'~' {
            font.add_glyph(
                char::from(byte),
                clonk_graphics::clonk_font::GlyphCell {
                    width: 6,
                    pixels: vec![clonk_graphics::Color::transparent(); 6 * 17],
                },
            );
        }
        font
    }

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
            unacknowledged: false,
        }
    }

    fn info_resources() -> RuntimeClientInfoResources {
        RuntimeClientInfoResources::new(
            "active",
            "inactive",
            "local",
            "remote",
            "host",
            "client",
            "%s %s %s %s (ID #%d):%s",
            "Addresses:",
            "  Data: %s (%s, %d ms)",
            "Connections: %s: %s (%s, %d ms)",
            "Addresses: none",
            "Connections: Not connected",
            "Unknown client ID #%d.",
        )
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
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(IntRect::new(20, 10, 800, 600), 20);
        assert_eq!((layout.bounds.w, layout.bounds.h), (600, 450));
        assert_eq!((layout.bounds.x, layout.bounds.y), (120, 85));
    }

    #[test]
    fn icon_button_visuals_keep_keyboard_and_pointer_activity_independent() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        dialog.focus = Some(RuntimeClientListFocus::Close);
        dialog.pointer = Some(GuiPoint::new(
            (layout.close_button.x + 1) as f32,
            (layout.close_button.y + 1) as f32,
        ));
        dialog.pointer_capture = Some(HitTarget::Close);

        assert_eq!(
            dialog.icon_button_state(HitTarget::Close, &layout, true, false),
            (true, false),
            "keyboard focus must not turn an inactive pointer capture into a press"
        );
        assert_eq!(
            dialog.icon_button_state(HitTarget::Close, &layout, false, true),
            (true, true)
        );
        assert_eq!(
            dialog.icon_button_state(HitTarget::Close, &layout, false, false),
            (false, false)
        );

        dialog.keyboard_press = Some((KeyCode::Enter, RuntimeClientListFocus::Close));
        assert_eq!(
            dialog.icon_button_state(HitTarget::Close, &layout, true, false),
            (true, true)
        );
    }

    #[test]
    fn client_row_and_escape_open_then_close_the_info_child() {
        let preferred = IntRect::new(0, 0, 640, 200);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let point = GuiPoint::new(
            (layout.list.x + 25) as f32,
            (layout.list.y + layout.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(point, preferred, 16));
        assert!(dialog.has_pointer_capture());
        assert_eq!(
            dialog.handle_pointer_up(point, preferred, 16),
            Some(RuntimeClientListAction::OpenInfo(7))
        );
        assert!(!dialog.has_pointer_capture());
        assert_eq!(dialog.info_client_id(), Some(7));
        assert_eq!(
            dialog.handle_key(KeyCode::Tab, false, preferred, 16),
            (true, None)
        );
        assert_eq!(
            dialog.focused(),
            Some(RuntimeClientListFocus::ClientList),
            "opening the modal info child retains the row focus underneath"
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
    fn static_info_uses_pipe_lines_without_a_client_or_live_snapshot() {
        let mut dialog = RuntimeClientListDialog::new_static_info(
            "Error Log",
            10,
            "oldest|middle||newest|",
            "&Close",
        );

        assert!(dialog.is_info_only());
        assert!(dialog.info_is_open());
        assert_eq!(dialog.info_client_id(), None);
        assert_eq!(dialog.info_caption(), "Error Log");
        assert_eq!(dialog.info_requested_line_count(), 10);
        assert_eq!(dialog.info_lines(), ["oldest", "middle", "newest"]);
        dialog.replace_snapshot_on_sec1(
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        assert!(dialog.info_is_open());
        assert_eq!(dialog.info_lines(), ["oldest", "middle", "newest"]);
        let preferred = IntRect::new(0, 0, 800, 600);
        let layout = dialog.info_layout(preferred, 16).expect("static layout");
        assert_eq!(layout.bounds.w, 620);
        assert_eq!(layout.caption.h, 23);
        assert_eq!(layout.close_button.y, layout.caption.y + 4);
        let bottom_close = layout.bottom_close_button.expect("bottom Close button");
        assert_eq!(bottom_close.w, 140);
        assert_eq!(bottom_close.h, 32);
        assert_eq!(
            bottom_close.y - (layout.text_window.y + layout.text_window.h),
            24
        );
        assert_eq!(
            dialog.handle_escape(true),
            Some(RuntimeClientListAction::CloseInfo)
        );
        assert!(!dialog.info_is_open());
    }

    #[test]
    fn static_info_bottom_close_and_enter_dismiss_the_modal() {
        let preferred = IntRect::new(0, 0, 800, 600);
        let mut dialog =
            RuntimeClientListDialog::new_static_info("Error Log", 10, "retained", "&Close");
        let button = dialog
            .info_layout(preferred, 16)
            .and_then(|layout| layout.bottom_close_button)
            .expect("bottom Close button");
        let point = GuiPoint::new(
            (button.x + button.w / 2) as f32,
            (button.y + button.h / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(point, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(point, preferred, 16),
            Some(RuntimeClientListAction::CloseInfo)
        );
        assert!(!dialog.info_is_open());

        let mut dialog =
            RuntimeClientListDialog::new_static_info("Error Log", 10, "retained", "&Close");
        assert_eq!(
            dialog.handle_key(KeyCode::Enter, false, preferred, 16),
            (true, Some(RuntimeClientListAction::CloseInfo))
        );
        assert!(!dialog.info_is_open());
    }

    #[test]
    fn main_and_info_title_drags_retain_independent_offsets_across_refresh() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let initial = dialog.layout(preferred, 16);
        let main_start = GuiPoint::new(
            (initial.caption.x + 8) as f32,
            (initial.caption.y + initial.caption.h / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(main_start, preferred, 16));
        assert!(dialog.has_positional_pointer_drag());
        let main_moved = GuiPoint::new(main_start.x + 37.0, main_start.y - 19.0);
        assert!(dialog.handle_pointer_move(main_moved, preferred, 16));
        let live_main = dialog.layout(preferred, 16);
        assert_eq!(live_main.bounds.x, initial.bounds.x + 37);
        assert_eq!(live_main.bounds.y, initial.bounds.y - 19);

        let main_released = GuiPoint::new(main_moved.x + 3.0, main_moved.y + 4.0);
        assert_eq!(dialog.handle_pointer_up(main_released, preferred, 16), None);
        assert!(!dialog.has_positional_pointer_drag());
        let retained_main = dialog.layout(preferred, 16);
        assert_eq!(retained_main.bounds.x, initial.bounds.x + 40);
        assert_eq!(retained_main.bounds.y, initial.bounds.y - 15);

        dialog.replace_snapshot(
            options(true),
            vec![row()],
            RuntimeClientListStatus {
                tick: 1,
                ..RuntimeClientListStatus::default()
            },
        );
        assert_eq!(dialog.layout(preferred, 16).bounds, retained_main.bounds);

        let current = dialog.layout(preferred, 16);
        let row_point = GuiPoint::new(
            (current.list.x + 25) as f32,
            (current.list.y + 2 + current.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(row_point, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(row_point, preferred, 16),
            Some(RuntimeClientListAction::OpenInfo(7))
        );
        let initial_info = dialog.info_layout(preferred, 16).expect("info layout");
        assert_eq!(
            initial_info.bounds.x + initial_info.bounds.w / 2,
            initial.bounds.x + initial.bounds.w / 2,
            "the separately centered info dialog must not inherit the main drag"
        );
        assert_eq!(
            initial_info.bounds.y + initial_info.bounds.h / 2,
            initial.bounds.y + initial.bounds.h / 2
        );

        let info_start = GuiPoint::new(
            (initial_info.caption.x + 8) as f32,
            (initial_info.caption.y + initial_info.caption.h / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(info_start, preferred, 16));
        let info_moved = GuiPoint::new(info_start.x - 22.0, info_start.y + 31.0);
        assert!(dialog.handle_pointer_move(info_moved, preferred, 16));
        let live_info = dialog
            .info_layout(preferred, 16)
            .expect("moved info layout");
        assert_eq!(live_info.bounds.x, initial_info.bounds.x - 22);
        assert_eq!(live_info.bounds.y, initial_info.bounds.y + 31);
        assert_eq!(dialog.layout(preferred, 16).bounds, retained_main.bounds);

        let info_released = GuiPoint::new(info_moved.x - 5.0, info_moved.y + 2.0);
        assert_eq!(dialog.handle_pointer_up(info_released, preferred, 16), None);
        let retained_info = dialog
            .info_layout(preferred, 16)
            .expect("retained info layout");
        assert_eq!(retained_info.bounds.x, initial_info.bounds.x - 27);
        assert_eq!(retained_info.bounds.y, initial_info.bounds.y + 33);

        dialog.replace_snapshot(
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        assert_eq!(
            dialog
                .info_layout(preferred, 16)
                .expect("info retained")
                .bounds,
            retained_info.bounds
        );
        assert_eq!(dialog.layout(preferred, 16).bounds, retained_main.bounds);

        let moved_close = GuiPoint::new(
            (retained_info.close_button.x + 1) as f32,
            (retained_info.close_button.y + 1) as f32,
        );
        assert!(dialog.handle_pointer_down(moved_close, preferred, 16));
        assert_eq!(
            dialog.handle_pointer_up(moved_close, preferred, 16),
            Some(RuntimeClientListAction::CloseInfo),
            "rendering and hit-testing must share the dragged info geometry"
        );
    }

    #[test]
    fn main_and_info_titles_bounce_one_pixel_per_draw_after_three_seconds() {
        let font = unit_width_font("W");
        let preferred = IntRect::new(0, 0, 240, 200);
        let mut dialog = RuntimeClientListDialog::new(
            "W".repeat(158),
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        )
        .with_info_caption("W".repeat(158));
        let layout = dialog.layout(preferred, font.line_height);
        assert_eq!(
            font.measure(&dialog.caption, true).0 + TITLE_LEFT_INDENT + TITLE_RIGHT_INDENT
                - layout.caption.w,
            3
        );
        let base = Instant::now();
        assert_eq!(
            dialog.caption_scroll_offset_at(base, &font, &layout.caption, DialogTitle::Main),
            0
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(
                base + TITLE_SCROLL_DELAY - Duration::from_millis(1),
                &font,
                &layout.caption,
                DialogTitle::Main,
            ),
            0
        );
        let outbound = base + TITLE_SCROLL_DELAY;
        assert_eq!(
            dialog.caption_scroll_offset_at(outbound, &font, &layout.caption, DialogTitle::Main),
            1
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(outbound, &font, &layout.caption, DialogTitle::Main),
            2
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(outbound, &font, &layout.caption, DialogTitle::Main),
            2,
            "the attempted far endpoint backs off and begins its three-second dwell"
        );

        dialog.info_client_id = Some(7);
        let info = dialog
            .info_layout(preferred, font.line_height)
            .expect("info layout");
        assert_eq!(
            font.measure(dialog.info_dialog.caption(), true).0
                + TITLE_LEFT_INDENT
                + TITLE_RIGHT_INDENT
                - info.caption.w,
            3
        );
        let info_base = outbound + Duration::from_secs(1);
        assert_eq!(
            dialog.caption_scroll_offset_at(info_base, &font, &info.caption, DialogTitle::Info,),
            0,
            "the info dialog owns an independent three-second clock"
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(
                info_base + TITLE_SCROLL_DELAY,
                &font,
                &info.caption,
                DialogTitle::Info,
            ),
            1
        );
        assert_eq!(
            dialog.caption_scroll_offset_at(
                info_base + TITLE_SCROLL_DELAY,
                &font,
                &info.caption,
                DialogTitle::Info,
            ),
            2
        );
    }

    #[test]
    fn title_and_close_tooltips_use_the_shared_mouse_delay_with_info_precedence() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        )
        .with_info_caption("Client information");
        let layout = dialog.layout(preferred, 16);
        let title_point = GuiPoint::new(
            (layout.caption.x + 8) as f32,
            (layout.caption.y + layout.caption.h / 2) as f32,
        );
        let base = Instant::now();
        let mut tracker = ClassicTooltipTracker::new_at(base);
        tracker.note_pointer_move_at(title_point, base);
        assert!(dialog.handle_pointer_move(title_point, preferred, 16));
        assert!(tracker
            .eligible_pointer_at(base + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1))
            .and_then(|point| dialog.tooltip_at(point, preferred, 16))
            .is_none());
        assert_eq!(
            tracker
                .eligible_pointer_at(base + CLASSIC_TOOLTIP_DELAY)
                .and_then(|point| dialog.tooltip_at(point, preferred, 16)),
            Some(StartupTooltip::text("Network"))
        );

        let close_point = GuiPoint::new(
            (layout.close_button.x + 1) as f32,
            (layout.close_button.y + 1) as f32,
        );
        let close_at = base + Duration::from_secs(1);
        tracker.note_pointer_move_at(close_point, close_at);
        assert!(dialog.handle_pointer_move(close_point, preferred, 16));
        assert_eq!(
            tracker
                .eligible_pointer_at(close_at + CLASSIC_TOOLTIP_DELAY)
                .and_then(|point| dialog.tooltip_at(point, preferred, 16)),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            "the close child wins its overlap with the caption"
        );

        dialog.info_client_id = Some(7);
        let info = dialog.info_layout(preferred, 16).expect("info layout");
        let info_title = GuiPoint::new(
            (info.caption.x + 8) as f32,
            (info.caption.y + info.caption.h / 2) as f32,
        );
        let info_at = close_at + Duration::from_secs(1);
        tracker.note_pointer_move_at(info_title, info_at);
        assert!(dialog.handle_pointer_move(info_title, preferred, 16));
        assert_eq!(
            tracker
                .eligible_pointer_at(info_at + CLASSIC_TOOLTIP_DELAY)
                .and_then(|point| dialog.tooltip_at(point, preferred, 16)),
            Some(StartupTooltip::text("Client information"))
        );

        let info_close = GuiPoint::new(
            (info.close_button.x + 1) as f32,
            (info.close_button.y + 1) as f32,
        );
        let info_close_at = info_at + Duration::from_secs(1);
        tracker.note_pointer_move_at(info_close, info_close_at);
        assert!(dialog.handle_pointer_move(info_close, preferred, 16));
        assert_eq!(
            tracker
                .eligible_pointer_at(info_close_at + CLASSIC_TOOLTIP_DELAY)
                .and_then(|point| dialog.tooltip_at(point, preferred, 16)),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE"))
        );
    }

    #[test]
    fn standalone_client_info_starts_on_the_requested_row_and_closes_as_info() {
        let mut dialog =
            RuntimeClientListDialog::new_info("Client information", row().client_id, Some(row()))
                .with_info_resources(info_resources());
        assert!(dialog.is_info_only());
        assert_eq!(dialog.info_client_id(), Some(7));
        assert_eq!(dialog.rows().len(), 1);
        assert_eq!(
            dialog.handle_escape(true),
            Some(RuntimeClientListAction::CloseInfo)
        );
        assert_eq!(dialog.info_client_id(), None);
    }

    // C4Network2ClientDlg::UpdateText resolves the client id on every update:
    // an unresolvable id prints IDS_NET_CLIENT_INFO_UNKNOWNID, and a host adds
    // the ` (!ack)` tail of IDS_NET_CLIENT_INFO_FORMAT while the client has not
    // acknowledged the current network status
    // (src/C4Network2Dialogs.cpp:54-71).
    #[test]
    fn client_info_text_shows_unknown_ids_and_the_host_ack_marker() {
        let resources = info_resources();
        assert_eq!(
            client_info_lines_for(&[], 12, &resources),
            vec!["Unknown client ID #12.".to_string()],
            "an id with no client renders the native fallback line"
        );

        let known = row();
        assert_eq!(
            client_info_lines_for(std::slice::from_ref(&known), 7, &resources)[0],
            "active remote client Remote (ID #7):"
        );

        let mut unacknowledged = known.clone();
        unacknowledged.unacknowledged = true;
        assert_eq!(
            client_info_lines_for(&[unacknowledged.clone()], 7, &resources)[0],
            "active remote client Remote (ID #7): (!ack)"
        );

        // The dialog is bound to an id, so a client that leaves keeps the
        // dialog open on the fallback text instead of closing it.
        let mut dialog = RuntimeClientListDialog::new_info("Client information", 7, Some(known))
            .with_info_resources(resources);
        assert_eq!(dialog.info_client_id(), Some(7));
        dialog.replace_snapshot(
            Vec::new(),
            vec![unacknowledged],
            RuntimeClientListStatus::default(),
        );
        assert_eq!(
            dialog.info_lines()[0],
            "active remote client Remote (ID #7): (!ack)"
        );
        dialog.replace_snapshot_on_sec1(Vec::new(), Vec::new(), RuntimeClientListStatus::default());
        assert_eq!(dialog.info_client_id(), Some(7), "the dialog stays open");
        assert_eq!(dialog.info_lines(), ["Unknown client ID #7.".to_string()]);
    }

    #[test]
    fn client_info_default_resources_keep_standalone_fallback_readable() {
        let dialog =
            RuntimeClientListDialog::new_info("Client information", row().client_id, Some(row()));
        assert_eq!(
            dialog.info_lines(),
            [
                "Active remote client Remote (ID #7):".to_string(),
                "Addresses: none".to_string(),
                "Connections: Not connected".to_string(),
            ]
        );
    }

    // `C4Network2ClientDlg::UpdateText` resolves every body template through
    // the active language table (`src/C4Network2Dialogs.cpp:65-105`).
    #[test]
    fn client_info_uses_active_resources_for_native_arguments() {
        let resources = RuntimeClientInfoResources::new(
            "AKTIV",
            "INAKTIV",
            "LOKAL",
            "FERN",
            "HOST",
            "CLIENT",
            "%s/%s/%s/%s/%d/%s",
            "ADRESSEN",
            "DATEN:%s:%s:%d",
            "VERBINDUNGEN:%s:%s:%s:%d",
            "KEINE-ADRESSEN:%d",
            "KEINE-VERBINDUNGEN",
            "UNBEKANNT:%d",
        );
        let mut row = row();
        row.host = true;
        row.local = true;
        row.activated = false;
        row.unacknowledged = true;
        row.addresses = vec!["addr.example:1234".to_string()];
        row.connections = vec![RuntimeConnectionRow {
            connection_id: 3,
            usage: "Data/Msg".to_string(),
            protocol: "UDP".to_string(),
            peer_address: "peer.example:5678".to_string(),
            packet_loss: 2,
            ping_ms: 17,
            lag_ms: 19,
            can_disconnect: false,
        }];

        let lines = client_info_lines_for(&[row], 7, &resources);
        assert_eq!(
            lines,
            [
                "INAKTIV/LOKAL/HOST/Remote/7/ (!ack)",
                "ADRESSEN",
                "  0: addr.example:1234",
                "VERBINDUNGEN:Msg/Data:UDP:peer.example:5678:17",
            ]
        );
        assert_eq!(
            client_info_lines_for(&[], 42, &resources),
            ["UNBEKANNT:42".to_string()]
        );
    }

    #[test]
    fn client_info_keeps_percent_literals_in_network_names() {
        let resources = info_resources();
        let mut row = row();
        row.name = "%s".to_string();
        row.nick = "%d".to_string();

        assert_eq!(
            client_info_lines_for(&[row], 7, &resources)[0],
            "active remote client %s (ID #7):"
        );
    }

    #[test]
    fn client_info_orders_message_before_separate_data_connection() {
        let resources = info_resources();
        let mut row = row();
        row.connections = vec![
            RuntimeConnectionRow {
                connection_id: 2,
                usage: "Data".to_string(),
                protocol: "TCP".to_string(),
                peer_address: "data.example:2222".to_string(),
                packet_loss: 0,
                ping_ms: 20,
                lag_ms: 20,
                can_disconnect: false,
            },
            RuntimeConnectionRow {
                connection_id: 1,
                usage: "Msg".to_string(),
                protocol: "UDP".to_string(),
                peer_address: "msg.example:1111".to_string(),
                packet_loss: 0,
                ping_ms: 10,
                lag_ms: 10,
                can_disconnect: false,
            },
        ];

        assert_eq!(
            client_info_lines_for(&[row], 7, &resources),
            [
                "active remote client Remote (ID #7):",
                "Addresses: none",
                "Connections: Msg: UDP (msg.example:1111, 10 ms)",
                "  Data: TCP (data.example:2222, 20 ms)",
            ]
        );
    }

    #[test]
    fn client_info_overflow_is_reachable_and_sec1_refresh_preserves_scroll() {
        let preferred = IntRect::new(0, 0, 640, 200);
        let font = tooltip_font();
        let mut overflow = row();
        overflow.addresses = (0..12)
            .map(|index| format!("10.0.0.{index}:111{index}"))
            .collect();
        overflow.connections = (0..6)
            .map(|index| RuntimeConnectionRow {
                connection_id: index,
                usage: format!("route-{index}"),
                protocol: "UDP".to_string(),
                peer_address: format!("192.0.2.{index}:222{index}"),
                packet_loss: index,
                ping_ms: 10 + index as i32,
                lag_ms: 10 + index as i32,
                can_disconnect: false,
            })
            .collect();
        let resources = info_resources();
        let all_lines = client_info_lines(&overflow, &resources);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![overflow.clone()],
            RuntimeClientListStatus::default(),
        )
        .with_info_resources(resources);
        let list = dialog.layout(preferred, font.line_height);
        let row_point = GuiPoint::new(
            (list.list.x + 25) as f32,
            (list.list.y + list.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(row_point, preferred, font.line_height));
        assert_eq!(
            dialog.handle_pointer_up(row_point, preferred, font.line_height),
            Some(RuntimeClientListAction::OpenInfo(7))
        );
        let roomy = IntRect::new(0, 0, 640, 480);
        assert_eq!(
            dialog
                .info_scroll_metrics(roomy, &font)
                .expect("unclamped scroll metrics")
                .viewport_height,
            10 * font.line_height,
            "the configurable line count fixes the unclamped viewport height"
        );
        let info = dialog
            .info_layout(preferred, font.line_height)
            .expect("standalone info layout");
        let point = GuiPoint::new(
            (info.text.x + 2) as f32,
            (info.text.y + info.text.h / 2) as f32,
        );
        let mut reached = std::collections::BTreeSet::new();

        loop {
            reached.extend(dialog.visible_info_lines(preferred, &font));
            let metrics = dialog
                .info_scroll_metrics(preferred, &font)
                .expect("scroll metrics");
            if metrics.scroll_y == metrics.max_scroll {
                assert!(metrics.max_scroll > 0, "fixture must overflow the viewport");
                break;
            }
            assert!(dialog.handle_wheel(point, -16, preferred, font.line_height));
        }
        assert!(
            all_lines.iter().all(|line| reached.contains(line)),
            "every generated address and connection line must become visible"
        );

        let retained_scroll = dialog
            .info_scroll_metrics(preferred, &font)
            .expect("retained scroll metrics")
            .scroll_y;
        overflow
            .connections
            .last_mut()
            .expect("tail connection")
            .peer_address = "refreshed-tail.example:9999".to_string();
        dialog.replace_snapshot_on_sec1(
            options(true),
            vec![overflow],
            RuntimeClientListStatus::default(),
        );
        assert_eq!(
            dialog
                .info_scroll_metrics(preferred, &font)
                .expect("refreshed scroll metrics")
                .scroll_y,
            retained_scroll,
            "the one-second refresh retains the absolute pixel offset"
        );
        assert!(dialog
            .visible_info_lines(preferred, &font)
            .iter()
            .any(|line| line.contains("refreshed-tail.example:9999")));
    }

    #[test]
    fn wheel_scroll_makes_an_initially_hidden_client_actionable() {
        let preferred = IntRect::new(0, 0, 320, 200);
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
            options(true),
            rows,
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let visible_rows = RuntimeClientListDialog::visible_list_row_count(&layout);
        assert!(visible_rows > 0 && visible_rows < dialog.rows().len());
        let hidden_client_id = dialog.rows()[visible_rows].client_id;
        let hidden_point = GuiPoint::new(
            (layout.list.x + layout.list.w - 3 - layout.icon_size / 2) as f32,
            (layout.list.y + 2 + visible_rows as i32 * layout.row_height + layout.row_height / 2)
                as f32,
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

    #[test]
    fn runtime_options_emit_mouse_combo_requests() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        for row_layout in &layout.option_rows {
            let option = dialog.option_rows()[row_layout.index].kind;
            let value = row_layout.value;
            let point = GuiPoint::new((value.x + 2) as f32, (value.y + 2) as f32);
            assert!(dialog.handle_pointer_down(point, preferred, 16));
            assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::OptionsList));
            assert_eq!(
                dialog.handle_pointer_up(point, preferred, 16),
                Some(RuntimeClientListAction::OptionSelectionRequested {
                    option,
                    anchor: GuiPoint::new(value.x as f32, (value.y + value.h) as f32),
                    minimum_width: value.w,
                })
            );
        }
        let kick_point = GuiPoint::new(
            (layout.list.x + layout.list.w - 3 - layout.icon_size / 2) as f32,
            (layout.list.y + 2 + layout.row_height / 2) as f32,
        );
        assert!(dialog.handle_pointer_down(kick_point, preferred, 16));
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::ClientList));
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(7))
        );
        dialog.pointer_left();

        let caption_reference = "A deliberately wide runtime-join caption";
        let mut client = RuntimeClientListDialog::new(
            "Network",
            options(false),
            vec![row()],
            RuntimeClientListStatus::default(),
        )
        .with_option_caption_reference(caption_reference);
        assert!(client.option_rows().iter().all(|option| !option.editable));
        assert!(!client
            .option_rows()
            .iter()
            .any(|option| option.kind == LobbyOptionKind::RuntimeJoin));
        let font = tooltip_font();
        client.measure_option_caption_width(&font);
        let client_layout = client.layout(preferred, 16);
        let first = client_layout.option_rows[0];
        let expected_caption_width =
            (font.measure(caption_reference, true).0 * 5 / 4).min((first.rect.w - 10).max(0));
        assert_eq!(first.value.x, first.rect.x + expected_caption_width + 8);
        let read_only = client_layout.option_rows[0].value;
        let point = GuiPoint::new((read_only.x + 2) as f32, (read_only.y + 2) as f32);
        assert!(client.handle_pointer_down(point, preferred, 16));
        assert_eq!(client.handle_pointer_up(point, preferred, 16), None);
    }

    #[test]
    fn runtime_options_auto_scrollbar_reaches_hidden_rows() {
        let preferred = IntRect::new(0, 0, 320, 200);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let initial = dialog.layout(preferred, 16);
        assert!(initial.option_rows.len() < dialog.option_rows().len());
        assert_eq!(initial.option_rows[0].rect.h, 22);
        assert_eq!(
            initial.option_rows[0].value.y,
            initial.option_rows[0].rect.y + 1
        );
        assert_eq!(
            initial.option_rows[0].rect.x + initial.option_rows[0].rect.w,
            initial.option_scrollbar.x
        );
        let down = GuiPoint::new(
            (initial.option_scrollbar.x + initial.option_scrollbar.w / 2) as f32,
            (initial.option_scrollbar.y + initial.option_scrollbar.h - SCROLLBAR_EXTENT / 2) as f32,
        );
        for _ in 0..2 {
            assert!(dialog.handle_pointer_down(down, preferred, 16));
            assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::OptionsList));
            assert_eq!(dialog.handle_pointer_up(down, preferred, 16), None);
        }
        let scrolled = dialog.layout(preferred, 16);
        let runtime_join = scrolled
            .option_rows
            .iter()
            .find(|row| row.index == 2)
            .expect("runtime join row should be reachable through the scrollbar");
        let point = GuiPoint::new(
            (runtime_join.value.x + 2) as f32,
            (runtime_join.value.y + 2) as f32,
        );
        assert!(dialog.handle_pointer_down(point, preferred, 16));
        assert!(matches!(
            dialog.handle_pointer_up(point, preferred, 16),
            Some(RuntimeClientListAction::OptionSelectionRequested {
                option: LobbyOptionKind::RuntimeJoin,
                ..
            })
        ));
    }

    #[test]
    fn tab_focuses_native_order_and_list_keys_select_every_entry() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut first = row();
        first.connections.push(RuntimeConnectionRow {
            connection_id: 42,
            usage: "Data".to_string(),
            protocol: "UDP".to_string(),
            peer_address: "127.0.0.1:1111".to_string(),
            packet_loss: 0,
            ping_ms: 12,
            lag_ms: 12,
            can_disconnect: true,
        });
        let mut second = row();
        second.client_id = 8;
        second.name = "Second".to_string();
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![first, second],
            RuntimeClientListStatus::default(),
        );

        assert_eq!(
            dialog.handle_key(KeyCode::Tab, false, preferred, 16),
            (true, None)
        );
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Close));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::OptionsList));
        assert_eq!(
            dialog.handle_key(KeyCode::Down, false, preferred, 16),
            (true, None)
        );
        assert_eq!(
            dialog.handle_key(KeyCode::Home, false, preferred, 16),
            (true, None)
        );
        assert_eq!(dialog.selected_client_id(), None);

        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::ClientList));
        assert_eq!(dialog.selected_client_id(), Some(7));
        dialog.handle_key(KeyCode::Down, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Connection {
                client_id: 7,
                connection_id: 42,
            })
        );
        dialog.handle_key(KeyCode::Down, false, preferred, 16);
        assert_eq!(dialog.selected_client_id(), Some(8));
        dialog.handle_key(KeyCode::Down, false, preferred, 16);
        assert_eq!(dialog.selected_client_id(), Some(8));
        dialog.handle_key(KeyCode::Up, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Connection {
                client_id: 7,
                connection_id: 42,
            })
        );
        dialog.handle_key(KeyCode::Tab, true, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::OptionsList));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::ClientList));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(
            dialog.focused(),
            Some(RuntimeClientListFocus::Disconnect {
                client_id: 7,
                connection_id: 42,
            })
        );
        assert_eq!(
            dialog.handle_key(KeyCode::Enter, false, preferred, 16),
            (true, None)
        );
        assert_eq!(
            dialog.handle_key_release(KeyCode::Enter),
            (
                true,
                Some(RuntimeClientListAction::Disconnect {
                    client_id: 7,
                    connection_id: 42,
                })
            )
        );
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Close));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::ClientList));
        dialog.handle_key(KeyCode::Up, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(7))
        );
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Mute(7)));
        assert_eq!(
            dialog.handle_key(KeyCode::Enter, false, preferred, 16),
            (true, None)
        );
        assert_eq!(
            dialog.handle_key_release(KeyCode::Enter),
            (true, Some(RuntimeClientListAction::ToggleMute(7)))
        );
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Activate(7)));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Kick(7)));
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::Close));

        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        dialog.handle_key(KeyCode::Tab, false, preferred, 16);
        assert_eq!(dialog.focused(), Some(RuntimeClientListFocus::ClientList));
        dialog.handle_key(KeyCode::End, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(8))
        );
        dialog.handle_key(KeyCode::Home, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(7))
        );
        dialog.handle_key(KeyCode::PageDown, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(8))
        );
        dialog.handle_key(KeyCode::PageUp, false, preferred, 16);
        assert_eq!(
            dialog.selected_entry(),
            Some(RuntimeClientListSelection::Client(7))
        );
    }

    #[test]
    fn name_tooltip_waits_exactly_and_contains_only_player_names() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut client = row();
        client.player_names = vec!["Alpha".to_string(), "Beta".to_string()];
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![client],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let label = client_label_rect(&dialog.rows()[0], &layout, layout.list.y + 2);
        let point = GuiPoint::new((label.x + 1) as f32, (label.y + 3) as f32);
        let font = tooltip_font();
        let started = Instant::now();
        dialog.handle_pointer_move_at(point, preferred, 16, started);
        assert_eq!(
            dialog.tooltip_state_at(
                started + CLASSIC_TOOLTIP_DELAY - std::time::Duration::from_millis(1),
                preferred,
                &font,
            ),
            None
        );
        assert_eq!(
            dialog
                .tooltip_state_at(started + CLASSIC_TOOLTIP_DELAY, preferred, &font)
                .map(|tooltip| tooltip.text),
            Some("Alpha, Beta".to_string())
        );
        dialog.note_non_pointer_input();
        assert_eq!(
            dialog.tooltip_state_at(started + CLASSIC_TOOLTIP_DELAY, preferred, &font),
            None
        );

        let margin_point = GuiPoint::new((label.x + 1) as f32, (label.y + 1) as f32);
        let margin_started = started + CLASSIC_TOOLTIP_DELAY;
        dialog.handle_pointer_move_at(margin_point, preferred, 16, margin_started);
        assert_eq!(
            dialog.tooltip_state_at(margin_started + CLASSIC_TOOLTIP_DELAY, preferred, &font,),
            None,
            "the native Label's two-pixel top margin owns no tooltip"
        );

        let status_point = GuiPoint::new((layout.list.x + 4) as f32, (label.y + 3) as f32);
        let moved = margin_started + CLASSIC_TOOLTIP_DELAY;
        dialog.handle_pointer_move_at(status_point, preferred, 16, moved);
        assert_eq!(
            dialog.tooltip_state_at(moved + CLASSIC_TOOLTIP_DELAY, preferred, &font),
            None
        );

        let mut short = row();
        short.name = "<c ff0000>A</c>".to_string();
        short.nick.clear();
        short.player_names = vec!["Alpha".to_string()];
        let mut short_dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![short],
            RuntimeClientListStatus::default(),
        );
        let short_layout = short_dialog.layout(preferred, 16);
        let short_label = client_label_rect(
            &short_dialog.rows()[0],
            &short_layout,
            short_layout.list.y + 2,
        );
        let short_text_width = font.measure(&short_dialog.rows()[0].label(), true).0;
        let blank_point = GuiPoint::new(
            (short_label.x + short_text_width + 2) as f32,
            (short_label.y + 3) as f32,
        );
        short_dialog.handle_pointer_move_at(blank_point, preferred, 16, started);
        assert_eq!(
            short_dialog.tooltip_state_at(started + CLASSIC_TOOLTIP_DELAY, preferred, &font),
            None,
            "blank row space to the right of the native Label owns no tooltip"
        );

        let mut empty = row();
        empty.player_names.clear();
        let mut empty_dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![empty],
            RuntimeClientListStatus::default(),
        );
        let empty_layout = empty_dialog.layout(preferred, 16);
        let empty_label = client_label_rect(
            &empty_dialog.rows()[0],
            &empty_layout,
            empty_layout.list.y + 2,
        );
        let empty_point = GuiPoint::new((empty_label.x + 1) as f32, (empty_label.y + 3) as f32);
        empty_dialog.handle_pointer_move_at(empty_point, preferred, 16, started);
        assert_eq!(
            empty_dialog.tooltip_state_at(started + CLASSIC_TOOLTIP_DELAY, preferred, &font),
            None
        );
    }

    #[test]
    fn option_tooltip_uses_the_native_option_row_text() {
        let preferred = IntRect::new(0, 0, 640, 480);
        let mut dialog = RuntimeClientListDialog::new(
            "Network",
            options(true),
            vec![row()],
            RuntimeClientListStatus::default(),
        );
        let layout = dialog.layout(preferred, 16);
        let option = dialog.option_rows()[0].tooltip.clone();
        let row = layout.option_rows[0].rect;
        let point = GuiPoint::new((row.x + 1) as f32, (row.y + 1) as f32);
        let started = Instant::now();
        dialog.handle_pointer_move_at(point, preferred, 16, started);
        assert_eq!(
            dialog
                .tooltip_state_at(started + CLASSIC_TOOLTIP_DELAY, preferred, &tooltip_font(),)
                .map(|tooltip| tooltip.text),
            Some(option)
        );
    }
}
