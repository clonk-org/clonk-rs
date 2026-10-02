//! Layout and rendering for the in-game conversation panel.

use std::time::{Duration, Instant};

use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};

use crate::classic_gui::{draw_clipped_text_with_markup, draw_engine_box, IntRect};
use crate::enhanced_chat::{ChatChannel, ChatMessage, EnhancedChat};
use crate::{ClonkFontSet, GuiPoint};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatPreferences {
    pub enabled: bool,
    pub text_size: u8,
    pub opacity: u8,
    pub duration_seconds: u32,
}

impl Default for ChatPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            text_size: 1,
            opacity: 85,
            duration_seconds: 12,
        }
    }
}

impl ChatPreferences {
    pub fn font<'a>(&self, fonts: &'a ClonkFontSet) -> &'a ClonkFont {
        match self.text_size {
            0 => &fonts.main_small,
            2 => &fonts.caption,
            _ => &fonts.text,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ChatLayout {
    pub bounds: IntRect,
    pub header: IntRect,
    /// Conversation only.
    pub chat_tab: IntRect,
    /// Conversation and game messages.
    pub all_tab: IntRect,
    /// "Show over game": recent messages over play while chat is closed.
    pub overlay_toggle: IntRect,
    pub settings_button: IntRect,
    pub close: IntRect,
    pub feed: IntRect,
    pub audience: IntRect,
    pub latest: IntRect,
    pub edit: IntRect,
    pub notice: IntRect,
}

/// Width below which the overlay toggle drops to its short label.
const OVERLAY_LABEL_WIDTH: i32 = 128;

impl ChatLayout {
    pub fn new(width: i32, height: i32, line_height: i32, expanded: bool) -> Self {
        let w = (width - 24).clamp(1, 560);
        let row = 22;
        let edit_height = line_height + 8;
        let h = if expanded {
            360
        } else {
            (line_height + 3) * 6 + 16
        };
        let h = h.min((height - 8).max(1));
        let x = 12.min((width - w).max(0));
        let y = (height - h - 28).max(0);
        let bounds = IntRect::new(x, y, w, h);
        let inner_x = x + 8;
        let inner_w = (w - 16).max(1);
        let header = IntRect::new(inner_x, y + 6, inner_w, row);
        let right = inner_x + inner_w;
        let chat_tab = IntRect::new(inner_x, header.y, 44, row);
        let all_tab = IntRect::new(chat_tab.x + chat_tab.w + 4, header.y, 34, row);
        let close = IntRect::new(right - 24, header.y, 24, row);
        let settings_button = IntRect::new(close.x - 4 - 66, header.y, 66, row);
        let tabs_end = all_tab.x + all_tab.w + 8;
        let overlay_width = if settings_button.x - 8 - OVERLAY_LABEL_WIDTH >= tabs_end {
            OVERLAY_LABEL_WIDTH
        } else {
            62
        };
        let overlay_toggle = IntRect::new(
            settings_button.x - 8 - overlay_width,
            header.y,
            overlay_width,
            row,
        );
        let notice = IntRect::new(inner_x, y + h - row * 2 - 6, inner_w, row * 2);
        let edit = IntRect::new(inner_x, notice.y - edit_height - 4, inner_w, edit_height);
        let audience = IntRect::new(inner_x, edit.y - row - 4, inner_w / 2, row);
        let latest = IntRect::new(
            inner_x + inner_w / 2,
            audience.y,
            inner_w - inner_w / 2,
            row,
        );
        let feed_top = if expanded {
            header.y + header.h + 6
        } else {
            y + 8
        };
        let feed_bottom = if expanded { audience.y - 4 } else { y + h - 6 };
        let feed = IntRect::new(inner_x, feed_top, inner_w, (feed_bottom - feed_top).max(0));
        Self {
            bounds,
            header,
            chat_tab,
            all_tab,
            overlay_toggle,
            settings_button,
            close,
            feed,
            audience,
            latest,
            edit,
            notice,
        }
    }
}

pub fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

pub struct ChatView<'a> {
    pub preferences: &'a ChatPreferences,
    pub expanded: bool,
    pub audience: &'a str,
    pub hint: &'a str,
    pub notice: &'a str,
    pub timestamps: bool,
    /// `General.UseWhiteIngameChat`: white message text with only the sender
    /// in player colour.
    pub white_text: bool,
    pub now: Instant,
}

fn text(
    surface: &mut Surface,
    font: &ClonkFont,
    rect: IntRect,
    label: &str,
    color: [u8; 4],
    gamma: Option<&GammaRamp>,
) {
    draw_clipped_text_with_markup(
        surface,
        font,
        rect.x,
        rect.y,
        label,
        color,
        TextAlign::Left,
        gamma,
        rect,
        false,
    );
}

fn fill(surface: &mut Surface, rect: IntRect, color: u32, gamma: Option<&GammaRamp>) {
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        rect.y + rect.h - 1,
        color,
        gamma,
    );
}

/// Wrap literal user text; markup in a player's message cannot change the panel.
/// Character fallback keeps long words visible on narrow screens.
pub fn wrap_text(font: &ClonkFont, text: &str, width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for character in text.chars() {
        if character == '\n' {
            lines.push(std::mem::take(&mut line));
            continue;
        }
        let mut candidate = line.clone();
        candidate.push(character);
        if !line.is_empty() && font.measure(&candidate, false).0 > width.max(1) {
            if let Some(space) = line.rfind(' ') {
                let remainder = line[space + 1..].to_string();
                lines.push(line[..space].to_string());
                line = remainder;
            } else {
                lines.push(std::mem::take(&mut line));
            }
        }
        line.push(character);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn render_chat(
    surface: &mut Surface,
    fonts: &ClonkFontSet,
    chat: &EnhancedChat,
    view: &ChatView<'_>,
    gamma: Option<&GammaRamp>,
) {
    if !view.expanded && chat.hidden {
        return;
    }
    let font = view.preferences.font(fonts);
    let ui_font = &fonts.mini;
    let layout = ChatLayout::new(
        surface.width() as i32,
        surface.height() as i32,
        font.line_height,
        view.expanded,
    );
    let lines = display_lines(font, chat, view, layout.feed);
    if !view.expanded && lines.is_empty() {
        return;
    }
    if view.expanded {
        let alpha = 255 - u32::from(view.preferences.opacity.min(100)) * 255 / 100;
        fill(surface, layout.bounds, (alpha << 24) | 0x101923, gamma);
        render_header(surface, ui_font, chat, &layout, gamma);
        fill(surface, layout.audience, 0x00304152, gamma);
        text(
            surface,
            ui_font,
            layout.audience,
            view.audience,
            [255; 4],
            gamma,
        );
        let latest = if chat.unread() > 0 {
            format!("{} new messages v", chat.unread())
        } else {
            if chat.is_scrolled() {
                "Back to latest ↓"
            } else {
                ""
            }
            .into()
        };
        text(
            surface,
            ui_font,
            layout.latest,
            &latest,
            [240, 208, 148, 255],
            gamma,
        );
        fill(surface, layout.edit, 0x00101a27, gamma);
        let notice = if chat.error.is_empty() {
            view.notice
        } else {
            &chat.error
        };
        let notice_lines = wrap_text(ui_font, notice, layout.notice.w);
        let notice_color = if chat.error.is_empty() {
            [238, 203, 148, 255]
        } else {
            [255, 166, 155, 255]
        };
        for (row, line) in notice_lines.iter().take(2).enumerate() {
            text(
                surface,
                ui_font,
                IntRect::new(
                    layout.notice.x,
                    layout.notice.y + row as i32 * 22,
                    layout.notice.w,
                    22,
                ),
                line,
                notice_color,
                gamma,
            );
        }
        if notice_lines.len() < 2 {
            text(
                surface,
                ui_font,
                IntRect::new(
                    layout.notice.x,
                    layout.notice.y + notice_lines.len() as i32 * 22,
                    layout.notice.w,
                    22,
                ),
                view.hint,
                [174, 195, 215, 255],
                gamma,
            );
        }
    }
    let row_height = font.line_height + 3;
    let bottom = layout.feed.y + layout.feed.h;
    for (index, line) in lines.iter().enumerate() {
        let y = bottom - (lines.len() - index) as i32 * row_height;
        let right = layout.feed.x + layout.feed.w;
        let mut x = layout.feed.x;
        for span in &line.spans {
            let rect = IntRect::new(x, y, (right - x).max(0), font.line_height + 2);
            outlined_text(surface, font, rect, &span.text, span.color, gamma);
            x += font.measure(&span.text, false).0 + font.h_space;
        }
    }
}

/// Tabs choose what the transcript shows; the check box, Settings and
/// close sit at the right like any window's controls.
fn render_header(
    surface: &mut Surface,
    font: &ClonkFont,
    chat: &EnhancedChat,
    layout: &ChatLayout,
    gamma: Option<&GammaRamp>,
) {
    for (rect, label, selected) in [
        (layout.chat_tab, "Chat", !chat.show_logs),
        (layout.all_tab, "All", chat.show_logs),
    ] {
        let color = if selected {
            [240, 215, 170, 255]
        } else {
            TIMESTAMP_COLOR
        };
        text(
            surface,
            font,
            IntRect::new(rect.x + 4, rect.y + 2, rect.w - 4, rect.h - 2),
            label,
            color,
            gamma,
        );
        if selected {
            fill(
                surface,
                IntRect::new(rect.x + 2, rect.y + rect.h - 2, rect.w - 4, 2),
                0x00e5c99e,
                gamma,
            );
        }
    }
    let toggle = layout.overlay_toggle;
    let check = IntRect::new(toggle.x, toggle.y + (toggle.h - 11) / 2, 11, 11);
    fill(surface, check, 0x00aec3d7, gamma);
    fill(
        surface,
        IntRect::new(check.x + 1, check.y + 1, check.w - 2, check.h - 2),
        0x00101923,
        gamma,
    );
    if !chat.hidden {
        fill(
            surface,
            IntRect::new(check.x + 3, check.y + 3, check.w - 6, check.h - 6),
            0x00e5c99e,
            gamma,
        );
    }
    let label = if toggle.w >= OVERLAY_LABEL_WIDTH {
        "Show over game"
    } else {
        "Show"
    };
    let label_x = check.x + check.w + 5;
    text(
        surface,
        font,
        IntRect::new(
            label_x,
            toggle.y + 2,
            toggle.x + toggle.w - label_x,
            toggle.h - 2,
        ),
        label,
        [206, 218, 230, 255],
        gamma,
    );
    for (rect, label) in [(layout.settings_button, "Settings"), (layout.close, "×")] {
        fill(surface, rect, 0x80304152, gamma);
        text(
            surface,
            font,
            IntRect::new(rect.x + 5, rect.y + 2, rect.w - 10, rect.h - 2),
            label,
            [206, 218, 230, 255],
            gamma,
        );
    }
}

fn outlined_text(
    surface: &mut Surface,
    font: &ClonkFont,
    rect: IntRect,
    label: &str,
    color: [u8; 4],
    gamma: Option<&GammaRamp>,
) {
    if label.is_empty() {
        return;
    }
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        text(
            surface,
            font,
            IntRect::new(rect.x + dx, rect.y + dy, rect.w, rect.h),
            label,
            [5, 9, 14, color[3]],
            gamma,
        );
    }
    text(surface, font, rect, label, color, gamma);
}

pub(crate) const TIMESTAMP_COLOR: [u8; 4] = [140, 152, 166, 255];
pub(crate) const WHITE_TEXT: [u8; 4] = [237, 242, 248, 255];
pub(crate) const ALLIES_COLOR: [u8; 4] = [120, 220, 150, 255];
pub(crate) const PRIVATE_COLOR: [u8; 4] = [236, 156, 236, 255];
const LOG_COLOR: [u8; 4] = [160, 175, 191, 255];

/// A run of one colour within a displayed line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSpan {
    pub text: String,
    pub color: [u8; 4],
}

pub struct ChatLine {
    pub spans: Vec<ChatSpan>,
}

impl ChatLine {
    fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn set_alpha(&mut self, alpha: u8) {
        for span in &mut self.spans {
            span.color[3] = alpha;
        }
    }

    /// Shorten the line until it fits `width` with a trailing ellipsis.
    fn ellipsize(&mut self, font: &ClonkFont, width: i32) {
        while font.measure(&format!("{}…", self.text()), false).0 > width
            && self.spans.iter().any(|span| !span.text.is_empty())
        {
            if let Some(span) = self
                .spans
                .iter_mut()
                .rev()
                .find(|span| !span.text.is_empty())
            {
                span.text.pop();
            }
        }
        self.spans.retain(|span| !span.text.is_empty());
        match self.spans.last_mut() {
            Some(span) => span.text.push('…'),
            None => self.spans.push(ChatSpan {
                text: "…".into(),
                color: WHITE_TEXT,
            }),
        }
    }
}

fn display_lines(
    font: &ClonkFont,
    chat: &EnhancedChat,
    view: &ChatView<'_>,
    feed: IntRect,
) -> Vec<ChatLine> {
    let capacity = (feed.h / (font.line_height + 3).max(1)).max(0) as usize;
    let messages = if view.expanded {
        chat.visible_messages()
    } else {
        chat.matching_messages()
    };
    let mut groups = Vec::new();
    let mut count = 0;
    for message in messages.into_iter().rev() {
        if !view.expanded
            && view.now.saturating_duration_since(message.received)
                >= Duration::from_secs(u64::from(view.preferences.duration_seconds))
        {
            continue;
        }
        let mut lines = message_lines(font, message, feed.w, view.timestamps, view.white_text);
        if !view.expanded {
            let remaining = Duration::from_secs(u64::from(view.preferences.duration_seconds))
                .saturating_sub(view.now.saturating_duration_since(message.received));
            let opacity = (remaining.as_secs_f32() / 2.0).clamp(0.0, 1.0);
            for line in &mut lines {
                line.set_alpha((opacity * 255.0).round() as u8);
            }
        }
        if !view.expanded && lines.len() > capacity.min(2) {
            lines.truncate(capacity.min(2));
            if let Some(last) = lines.last_mut() {
                last.ellipsize(font, feed.w);
            }
        }
        if !view.expanded && count + lines.len() > capacity {
            break;
        }
        count += lines.len();
        groups.push(lines);
        if count >= capacity + if view.expanded { chat.line_offset() } else { 0 } {
            break;
        }
    }
    let mut lines: Vec<_> = groups.into_iter().rev().flatten().collect();
    if view.expanded {
        lines.truncate(lines.len().saturating_sub(chat.line_offset()));
        let start = lines.len().saturating_sub(capacity);
        lines.drain(..start);
    }
    lines
}

/// One message as coloured spans: a receding timestamp, a channel tag in its
/// channel's colour, the sender in their player colour, then the message in
/// white or, without white chat, in the sender's colour as classic chat
/// shows it.
pub fn message_lines(
    font: &ClonkFont,
    message: &ChatMessage,
    width: i32,
    timestamps: bool,
    white_text: bool,
) -> Vec<ChatLine> {
    let log = message.channel == ChatChannel::Log;
    let sender_color = if log {
        LOG_COLOR
    } else {
        [message.color[0], message.color[1], message.color[2], 255]
    };
    let (tag, tag_color) = match message.channel {
        ChatChannel::Everyone => ("", sender_color),
        ChatChannel::Allies => ("[Allies] ", ALLIES_COLOR),
        ChatChannel::Private => ("[Private] ", PRIVATE_COLOR),
        ChatChannel::Action => ("* ", sender_color),
        ChatChannel::Log => ("[Game] ", LOG_COLOR),
    };
    let stamp = if timestamps && !message.timestamp.is_empty() {
        format!("{} ", message.timestamp)
    } else {
        String::new()
    };
    let sender = match (message.sender.is_empty(), &message.channel) {
        (true, _) => String::new(),
        // An action reads as a sentence: "* Ada waves".
        (false, ChatChannel::Action) => format!("{} ", message.sender),
        (false, _) => format!("{}: ", message.sender),
    };
    let body_color = match (log, white_text) {
        (true, _) => LOG_COLOR,
        (false, true) => WHITE_TEXT,
        (false, false) => sender_color,
    };
    let segments = [
        (stamp, TIMESTAMP_COLOR),
        (tag.to_string(), tag_color),
        (sender, sender_color),
        (message.text.clone(), body_color),
    ];
    let combined: String = segments.iter().map(|(text, _)| text.as_str()).collect();
    let mut consumed = 0;
    wrap_text(font, &combined, width)
        .into_iter()
        .map(|line| {
            // Wrapping removes the separating space or newline, never player markup.
            let start = consumed + combined[consumed..].find(&line).unwrap_or(0);
            let end = start + line.len();
            consumed = end;
            let mut offset = 0;
            let spans = segments
                .iter()
                .filter_map(|(text, color)| {
                    let (from, to) = (offset, offset + text.len());
                    offset = to;
                    let (from, to) = (from.max(start), to.min(end));
                    (from < to).then(|| ChatSpan {
                        text: combined[from..to].into(),
                        color: *color,
                    })
                })
                .collect();
            ChatLine { spans }
        })
        .collect()
}

pub fn render_audience_picker(
    surface: &mut Surface,
    fonts: &ClonkFontSet,
    preferences: &ChatPreferences,
    labels: &[String],
    offset: usize,
    gamma: Option<&GammaRamp>,
) {
    let layout = ChatLayout::new(
        surface.width() as i32,
        surface.height() as i32,
        preferences.font(fonts).line_height,
        true,
    );
    fill(surface, layout.feed, 0x001b2b3d, gamma);
    for (row, label) in labels
        .iter()
        .skip(offset)
        .take((layout.feed.h / 22).max(0) as usize)
        .enumerate()
    {
        text(
            surface,
            &fonts.mini,
            IntRect::new(
                layout.feed.x + 4,
                layout.feed.y + row as i32 * 22,
                layout.feed.w - 8,
                22,
            ),
            label,
            [230, 237, 245, 255],
            gamma,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_names_leave_room_for_the_preview_ellipsis() {
        let fonts = crate::test_support::endeavour_font_set();
        let preferences = ChatPreferences::default();
        let mut chat = EnhancedChat::default();
        chat.push(ChatMessage::conversation(
            &"Ægir".repeat(30),
            ChatChannel::Private,
            "Meet at the bridge.",
        ));
        let view = ChatView {
            preferences: &preferences,
            expanded: false,
            audience: "Everyone",
            hint: "",
            notice: "",
            timestamps: false,
            white_text: true,
            now: Instant::now(),
        };
        let lines = display_lines(&fonts.text, &chat, &view, IntRect::new(0, 0, 100, 200));
        assert_eq!(lines.len(), 2);
        assert!(lines[1].text().ends_with('…'));
        for line in lines {
            assert!(fonts.text.measure(&line.text(), false).0 <= 100);
        }
    }

    #[test]
    fn recent_messages_fade_individually_and_remain_in_history() {
        let fonts = crate::test_support::endeavour_font_set();
        let preferences = ChatPreferences::default();
        let start = Instant::now();
        let mut chat = EnhancedChat::default();
        for second in 0..6 {
            let mut message = ChatMessage::conversation(
                "Ada",
                ChatChannel::Everyone,
                &format!("Message {second}"),
            );
            message.received = start + Duration::from_secs(second);
            chat.push(message);
        }
        let feed = ChatLayout::new(640, 480, fonts.text.line_height, false).feed;
        let mut view = ChatView {
            preferences: &preferences,
            expanded: false,
            audience: "Everyone",
            hint: "",
            notice: "",
            timestamps: false,
            white_text: true,
            now: start + Duration::from_secs(11),
        };
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 6);
        let alpha = |line: &ChatLine| line.spans[0].color[3];
        assert!(alpha(&lines[0]) > 0 && alpha(&lines[0]) < alpha(&lines[1]));
        assert_eq!(alpha(&lines[5]), 255);
        view.now = start + Duration::from_secs(13);
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].text(), "Ada: Message 2");
        view.expanded = true;
        view.now = start + Duration::from_secs(60);
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().all(|line| alpha(line) == 255));
    }

    #[test]
    fn sender_channel_and_timestamp_stand_apart_from_the_message() {
        let fonts = crate::test_support::endeavour_font_set();
        let mut message = ChatMessage::conversation("Ada", ChatChannel::Allies, "Ready?");
        message.color = [220, 60, 60, 255];
        message.timestamp = "[10:53:53]".into();
        let spans = |white_text| {
            message_lines(&fonts.text, &message, 500, true, white_text)
                .remove(0)
                .spans
        };

        let white = spans(true);
        let texts: Vec<_> = white.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["[10:53:53] ", "[Allies] ", "Ada: ", "Ready?"]);
        assert_eq!(white[0].color, TIMESTAMP_COLOR, "timestamps recede");
        assert_eq!(white[1].color, ALLIES_COLOR, "a channel has one colour");
        assert_eq!(
            white[2].color,
            [220, 60, 60, 255],
            "the sender keeps their colour"
        );
        assert_eq!(white[3].color, WHITE_TEXT);
        // Without white chat the message takes its sender's colour, as
        // classic chat does.
        assert_eq!(spans(false)[3].color, [220, 60, 60, 255]);
    }

    #[test]
    fn a_wrapped_message_keeps_its_colour_on_every_line() {
        let fonts = crate::test_support::endeavour_font_set();
        let mut message = ChatMessage::conversation(
            "Ada",
            ChatChannel::Private,
            "Meet at the lift and bring a shovel and two flints for the rock",
        );
        message.color = [220, 60, 60, 255];
        let lines = message_lines(&fonts.text, &message, 160, false, true);
        assert!(lines.len() > 1);
        assert_eq!(lines[0].spans[0].color, PRIVATE_COLOR);
        for line in &lines[1..] {
            assert!(line.spans.iter().all(|span| span.color == WHITE_TEXT));
        }
    }

    #[test]
    fn short_messages_keep_the_sender_and_body_on_one_line() {
        let fonts = crate::test_support::endeavour_font_set();
        let message = ChatMessage::conversation("Ada", ChatChannel::Everyone, "Ready?");
        assert_eq!(
            message_lines(&fonts.text, &message, 500, false, true).len(),
            1
        );
    }

    #[test]
    fn recent_chat_leaves_the_game_visible_outside_message_text() {
        let fonts = crate::test_support::endeavour_font_set();
        let preferences = ChatPreferences::default();
        let mut chat = EnhancedChat::default();
        chat.push(ChatMessage::conversation(
            "Ada",
            ChatChannel::Everyone,
            "Ready?",
        ));
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        surface.fill(clonk_graphics::Color::new(190, 210, 230, 255));
        let view = ChatView {
            preferences: &preferences,
            expanded: false,
            audience: "Everyone",
            hint: "",
            notice: "",
            timestamps: false,
            white_text: true,
            now: Instant::now(),
        };
        render_chat(&mut surface, &fonts, &chat, &view, None);
        if let Some(directory) = std::env::var_os("CLONK_CHAT_CAPTURE_DIR") {
            std::fs::create_dir_all(&directory).expect("create capture directory");
            crate::test_support::write_ppm(
                &surface,
                std::path::PathBuf::from(directory).join("chat-bright.ppm"),
            );
        }
        let layout = ChatLayout::new(640, 480, preferences.font(&fonts).line_height, false);
        assert_eq!(
            surface.get_pixel(
                (layout.bounds.x + layout.bounds.w - 2) as u32,
                (layout.bounds.y + 2) as u32
            ),
            Some(clonk_graphics::Color::new(190, 210, 230, 255))
        );
    }

    #[test]
    fn compact_panel_shows_new_messages_even_when_history_is_scrolled() {
        let fonts = crate::test_support::endeavour_font_set();
        let preferences = ChatPreferences::default();
        let mut chat = EnhancedChat::default();
        for message in ["first", "second", "newest"] {
            chat.push(ChatMessage::conversation(
                "Ada",
                ChatChannel::Everyone,
                message,
            ));
        }
        let view = ChatView {
            preferences: &preferences,
            expanded: false,
            audience: "Everyone",
            hint: "",
            notice: "",
            timestamps: false,
            white_text: true,
            now: Instant::now(),
        };
        let mut latest = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_chat(&mut latest, &fonts, &chat, &view, None);
        chat.scroll(true);
        let mut scrolled = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        render_chat(&mut scrolled, &fonts, &chat, &view, None);
        assert!(
            latest.pixels() == scrolled.pixels(),
            "compact view must follow new messages"
        );
    }

    #[test]
    fn composer_and_controls_fit_small_screens_and_large_text() {
        for (width, height) in [(320, 200), (640, 480), (1280, 720)] {
            for line_height in [22, 28, 36] {
                let layout = ChatLayout::new(width, height, line_height, true);
                let header = [
                    layout.chat_tab,
                    layout.all_tab,
                    layout.overlay_toggle,
                    layout.settings_button,
                    layout.close,
                ];
                for rect in [layout.bounds, layout.edit, layout.audience]
                    .into_iter()
                    .chain(header)
                {
                    assert!(rect.x >= 0 && rect.y >= 0 && rect.w > 0 && rect.h > 0);
                    assert!(rect.x + rect.w <= width && rect.y + rect.h <= height);
                }
                for pair in header.windows(2) {
                    assert!(
                        pair[0].x + pair[0].w <= pair[1].x,
                        "header controls overlap at {width}x{height}"
                    );
                }
                assert!(layout.feed.y + layout.feed.h <= layout.audience.y);
                assert!(layout.edit.y + layout.edit.h <= layout.notice.y);
            }
        }
    }
}
