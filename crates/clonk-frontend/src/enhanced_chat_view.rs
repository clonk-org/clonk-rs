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
            0 => &fonts.text,
            2 => &fonts.title,
            _ => &fonts.caption,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ChatLayout {
    pub bounds: IntRect,
    pub header: IntRect,
    pub filter: IntRect,
    pub feed: IntRect,
    pub audience: IntRect,
    pub latest: IntRect,
    pub edit: IntRect,
    pub notice: IntRect,
    pub settings: IntRect,
}

impl ChatLayout {
    pub fn new(width: i32, height: i32, line_height: i32, expanded: bool) -> Self {
        let w = (width - 24).clamp(1, 720);
        let row = 22;
        let edit_height = line_height + 8;
        let h = if expanded {
            400
        } else {
            row + line_height * 4 + 12
        };
        let h = h.min((height - 8).max(1));
        let x = 12.min((width - w).max(0));
        let y = (height - h - 28).max(0);
        let bounds = IntRect::new(x, y, w, h);
        let inner_x = x + 8;
        let inner_w = (w - 16).max(1);
        let header = IntRect::new(inner_x, y + 6, inner_w, row);
        let filter = IntRect::new(x + w / 2, header.y, w / 2 - 8, row);
        let settings = IntRect::new(inner_x, y + h - row - 6, inner_w, row);
        let notice = IntRect::new(inner_x, settings.y - row * 2, inner_w, row * 2);
        let edit = IntRect::new(inner_x, notice.y - edit_height - 4, inner_w, edit_height);
        let audience = IntRect::new(inner_x, edit.y - row - 4, inner_w / 2, row);
        let latest = IntRect::new(
            inner_x + inner_w / 2,
            audience.y,
            inner_w - inner_w / 2,
            row,
        );
        let feed_top = header.y + header.h + 6;
        let feed_bottom = if expanded { audience.y - 4 } else { y + h - 6 };
        let feed = IntRect::new(inner_x, feed_top, inner_w, (feed_bottom - feed_top).max(0));
        Self {
            bounds,
            header,
            filter,
            feed,
            audience,
            latest,
            edit,
            notice,
            settings,
        }
    }

    pub fn setting_cell(&self, index: i32) -> IntRect {
        let left = self.settings.x + self.settings.w * index / 4;
        let right = self.settings.x + self.settings.w * (index + 1) / 4;
        IntRect::new(left, self.settings.y, right - left, self.settings.h)
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
    let font = view.preferences.font(fonts);
    let ui_font = &fonts.mini;
    let layout = ChatLayout::new(
        surface.width() as i32,
        surface.height() as i32,
        font.line_height,
        view.expanded,
    );
    let messages = if view.expanded {
        chat.visible_messages()
    } else {
        chat.matching_messages()
    };
    let mut messages: Vec<_> = messages
        .into_iter()
        .filter(|message| {
            view.expanded
                || view.now.saturating_duration_since(message.received)
                    < Duration::from_secs(u64::from(view.preferences.duration_seconds))
        })
        .collect();
    let capacity = (layout.feed.h / font.line_height.max(1)).max(0) as usize;
    let needed = capacity.saturating_add(chat.line_offset()).max(1);
    if messages.len() > needed {
        messages.drain(..messages.len() - needed);
    }
    if !view.expanded && messages.is_empty() {
        return;
    }
    let alpha = 255 - u32::from(view.preferences.opacity.min(100)) * 255 / 100;
    fill(surface, layout.bounds, (alpha << 24) | 0x101923, gamma);
    fill(
        surface,
        IntRect::new(layout.bounds.x, layout.bounds.y, 3, layout.bounds.h),
        0x00cca875,
        gamma,
    );
    text(
        surface,
        ui_font,
        layout.header,
        if view.expanded {
            "CHAT"
        } else {
            "CHAT  ·  Enter to reply"
        },
        [229, 201, 158, 255],
        gamma,
    );
    if view.expanded {
        text(
            surface,
            ui_font,
            layout.filter,
            if chat.show_logs {
                "Chat + game log"
            } else {
                "Conversations"
            },
            [177, 198, 220, 255],
            gamma,
        );
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
            "Latest v  ·  PgUp / PgDn".into()
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
                IntRect::new(layout.notice.x, layout.notice.y + 22, layout.notice.w, 22),
                view.hint,
                [174, 195, 215, 255],
                gamma,
            );
        }
        let size = ["S", "M", "L"][usize::from(view.preferences.text_size.min(2))];
        let labels = [
            format!("Text: {size}"),
            format!("Back: {}%", view.preferences.opacity),
            format!("Show: {}s", view.preferences.duration_seconds),
            format!("Time: {}", if view.timestamps { "On" } else { "Off" }),
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
    // Store the header separately so names retain color while bodies remain readable.
    let mut lines = Vec::new();
    for message in messages {
        lines.extend(message_lines(font, message, layout.feed.w, view.timestamps));
    }
    if view.expanded {
        lines.truncate(lines.len().saturating_sub(chat.line_offset()));
    }
    let capacity = (layout.feed.h / font.line_height.max(1)).max(0) as usize;
    let mut start = lines.len().saturating_sub(capacity);
    if !view.expanded
        && start > 0
        && lines
            .get(start)
            .is_some_and(|(line, _)| line.starts_with(' '))
    {
        if let Some(next_header) = lines[start..]
            .iter()
            .position(|(line, _)| !line.starts_with(' '))
        {
            start += next_header;
        } else if capacity > 1 {
            // A long newest message still identifies its sender in compact mode.
            let header = lines[..start]
                .iter()
                .rfind(|(line, _)| !line.starts_with(' '))
                .cloned();
            if let Some(header) = header {
                lines[start] = header;
            }
        }
    }
    for (index, (line, color)) in lines[start..].iter().enumerate() {
        let rect = IntRect::new(
            layout.feed.x,
            layout.feed.y + index as i32 * font.line_height,
            layout.feed.w,
            font.line_height,
        );
        text(surface, font, rect, line, *color, gamma);
    }
}

pub fn message_lines(
    font: &ClonkFont,
    message: &ChatMessage,
    width: i32,
    timestamps: bool,
) -> Vec<(String, [u8; 4])> {
    let mut lines = Vec::new();
    let channel = match message.channel {
        ChatChannel::Everyone => "All",
        ChatChannel::Allies => "Allies",
        ChatChannel::Private => "Private",
        ChatChannel::Action => "Action",
        ChatChannel::Log => "Game",
    };
    let stamp = if timestamps {
        format!("{} ", message.timestamp)
    } else {
        String::new()
    };
    let header = format!("{stamp}[{channel}] {}", message.sender);
    let color = if message.channel == ChatChannel::Log {
        [160, 175, 191, 255]
    } else {
        // Pastel name colors remain legible even for dark player colors.
        [
            160 + (u16::from(message.color[0]) * 95 / 255) as u8,
            160 + (u16::from(message.color[1]) * 95 / 255) as u8,
            160 + (u16::from(message.color[2]) * 95 / 255) as u8,
            255,
        ]
    };
    for line in wrap_text(font, &header, width) {
        lines.push((line, color));
    }
    for line in wrap_text(font, &message.text, width - 8) {
        lines.push((format!(" {line}"), [237, 242, 248, 255]));
    }
    lines
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
