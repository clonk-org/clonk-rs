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
    pub filter: IntRect,
    pub options: IntRect,
    pub hide: IntRect,
    pub close: IntRect,
    pub feed: IntRect,
    pub audience: IntRect,
    pub latest: IntRect,
    pub edit: IntRect,
    pub notice: IntRect,
    pub settings: IntRect,
}

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
        let close = IntRect::new(right - 24, header.y, 24, row);
        let hide = IntRect::new(right - 76, header.y, 48, row);
        let options = IntRect::new(right - 148, header.y, 68, row);
        let filter = IntRect::new(right - 220, header.y, 68, row);
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
        let settings = IntRect::new(
            inner_x,
            feed_top,
            inner_w,
            (edit.y - feed_top - 4).clamp(0, 52),
        );
        Self {
            bounds,
            header,
            filter,
            options,
            hide,
            close,
            feed,
            audience,
            latest,
            edit,
            notice,
            settings,
        }
    }

    pub fn setting_cell(&self, index: i32) -> IntRect {
        let column = index % 2;
        let row = index / 2;
        let width = self.settings.w / 2;
        let height = self.settings.h / 2;
        IntRect::new(
            self.settings.x + column * width,
            self.settings.y + row * height,
            width - 4,
            height - 2,
        )
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
        text(
            surface,
            ui_font,
            layout.header,
            "Chat",
            [229, 201, 158, 255],
            gamma,
        );
        for (rect, label, selected) in [
            (
                layout.filter,
                if chat.show_logs {
                    "Log: on"
                } else {
                    "Log: off"
                },
                chat.show_logs,
            ),
            (layout.options, "Options", chat.options_open),
            (layout.hide, "Hide", false),
            (layout.close, "×", false),
        ] {
            fill(
                surface,
                rect,
                if selected { 0x003e4c5c } else { 0x80304152 },
                gamma,
            );
            text(
                surface,
                ui_font,
                IntRect::new(rect.x + 5, rect.y + 2, rect.w - 10, rect.h - 2),
                label,
                [206, 218, 230, 255],
                gamma,
            );
        }
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
        let rect = IntRect::new(layout.feed.x, y, layout.feed.w, font.line_height + 2);
        let prefix_width = font.measure(&line.prefix, false).0
            + if line.prefix.is_empty() || line.body.is_empty() {
                0
            } else {
                font.h_space
            };
        outlined_text(surface, font, rect, &line.prefix, line.color, gamma);
        outlined_text(
            surface,
            font,
            IntRect::new(
                rect.x + prefix_width,
                y,
                (rect.w - prefix_width).max(0),
                rect.h,
            ),
            &line.body,
            [237, 242, 248, line.color[3]],
            gamma,
        );
    }
    if view.expanded && chat.options_open {
        fill(surface, layout.settings, 0x001a2532, gamma);
        let size = ["Small", "Medium", "Large"][usize::from(view.preferences.text_size.min(2))];
        let labels = [
            format!("Text: {size}"),
            format!("History: {}%", view.preferences.opacity),
            format!("Keep: {}s", view.preferences.duration_seconds),
            format!("Timestamps: {}", if view.timestamps { "On" } else { "Off" }),
        ];
        for (index, label) in labels.iter().enumerate() {
            let cell = layout.setting_cell(index as i32);
            fill(surface, cell, 0x00304152, gamma);
            text(
                surface,
                ui_font,
                IntRect::new(cell.x + 3, cell.y, cell.w - 6, cell.h),
                label,
                [206, 218, 230, 255],
                gamma,
            );
        }
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

pub struct ChatLine {
    prefix: String,
    body: String,
    color: [u8; 4],
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
        let mut lines = message_lines(font, message, feed.w, view.timestamps);
        if !view.expanded {
            let remaining = Duration::from_secs(u64::from(view.preferences.duration_seconds))
                .saturating_sub(view.now.saturating_duration_since(message.received));
            let opacity = (remaining.as_secs_f32() / 2.0).clamp(0.0, 1.0);
            for line in &mut lines {
                line.color[3] = (opacity * 255.0).round() as u8;
            }
        }
        if !view.expanded && lines.len() > capacity.min(2) {
            lines.truncate(capacity.min(2));
            if let Some(last) = lines.last_mut() {
                while font
                    .measure(&format!("{}{}…", last.prefix, last.body), false)
                    .0
                    > feed.w
                    && !(last.body.is_empty() && last.prefix.is_empty())
                {
                    if last.body.pop().is_none() {
                        last.prefix.pop();
                    }
                }
                last.body.push('…');
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

/// Names and bodies share a row, while remaining separate literal-color spans.
pub fn message_lines(
    font: &ClonkFont,
    message: &ChatMessage,
    width: i32,
    timestamps: bool,
) -> Vec<ChatLine> {
    let channel = match message.channel {
        ChatChannel::Everyone => "",
        ChatChannel::Allies => "[Allies] ",
        ChatChannel::Private => "[Private] ",
        ChatChannel::Action => "* ",
        ChatChannel::Log => "[Game] ",
    };
    let stamp = if timestamps && !message.timestamp.is_empty() {
        format!("{} ", message.timestamp)
    } else {
        String::new()
    };
    let header = if message.sender.is_empty() {
        format!("{stamp}{channel}")
    } else {
        format!("{stamp}{channel}{}: ", message.sender)
    };
    let color = if message.channel == ChatChannel::Log {
        [160, 175, 191, 255]
    } else {
        [
            160 + (u16::from(message.color[0]) * 95 / 255) as u8,
            160 + (u16::from(message.color[1]) * 95 / 255) as u8,
            160 + (u16::from(message.color[2]) * 95 / 255) as u8,
            255,
        ]
    };
    let combined = format!("{header}{}", message.text);
    let mut consumed = 0;
    wrap_text(font, &combined, width)
        .into_iter()
        .map(|line| {
            // Wrapping removes the separating space or newline, never player markup.
            let start = consumed + combined[consumed..].find(&line).unwrap_or(0);
            let prefix_len = header.len().saturating_sub(start).min(line.len());
            consumed = start + line.len();
            ChatLine {
                prefix: line[..prefix_len].into(),
                body: line[prefix_len..].into(),
                color,
            }
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
            now: Instant::now(),
        };
        let lines = display_lines(&fonts.text, &chat, &view, IntRect::new(0, 0, 100, 200));
        assert_eq!(lines.len(), 2);
        assert!(lines[1].body.ends_with('…'));
        for line in lines {
            assert!(
                fonts
                    .text
                    .measure(&format!("{}{}", line.prefix, line.body), false)
                    .0
                    <= 100
            );
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
            now: start + Duration::from_secs(11),
        };
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 6);
        assert!(lines[0].color[3] > 0 && lines[0].color[3] < lines[1].color[3]);
        assert_eq!(lines[5].color[3], 255);
        view.now = start + Duration::from_secs(13);
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].body, "Message 2");
        view.expanded = true;
        view.now = start + Duration::from_secs(60);
        let lines = display_lines(&fonts.text, &chat, &view, feed);
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().all(|line| line.color[3] == 255));
    }

    #[test]
    fn short_messages_keep_the_sender_and_body_on_one_line() {
        let fonts = crate::test_support::endeavour_font_set();
        let message = ChatMessage::conversation("Ada", ChatChannel::Everyone, "Ready?");
        assert_eq!(message_lines(&fonts.text, &message, 500, false).len(), 1);
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
                for rect in [layout.bounds, layout.edit, layout.audience, layout.settings] {
                    assert!(rect.x >= 0 && rect.y >= 0 && rect.w > 0 && rect.h > 0);
                    assert!(rect.x + rect.w <= width && rect.y + rect.h <= height);
                }
                assert!(layout.feed.y + layout.feed.h <= layout.audience.y);
                assert!(layout.edit.y + layout.edit.h <= layout.notice.y);
            }
        }
    }
}
