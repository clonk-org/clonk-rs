//! Optional local presentation for game chat. No simulation or routing state.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatChannel {
    Everyone,
    Allies,
    Private,
    Action,
    Log,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChatAudience {
    #[default]
    Everyone,
    Allies,
    Private(i32),
    Say,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub id: u64,
    pub sender: String,
    pub channel: ChatChannel,
    pub text: String,
    pub color: [u8; 4],
    pub timestamp: String,
    pub received: Instant,
}

impl ChatMessage {
    pub fn conversation(sender: &str, channel: ChatChannel, text: &str) -> Self {
        Self {
            id: 0,
            sender: sender.into(),
            channel,
            text: text.into(),
            color: [180, 215, 255, 255],
            timestamp: String::new(),
            received: Instant::now(),
        }
    }
}

#[derive(Default)]
pub struct EnhancedChat {
    messages: VecDeque<ChatMessage>,
    next_id: u64,
    anchor: Option<u64>,
    line_offset: usize,
    unread: usize,
    pub show_logs: bool,
    pub audience: ChatAudience,
    drafts: BTreeMap<ChatAudience, String>,
    sent: BTreeMap<ChatAudience, VecDeque<String>>,
    history_index: Option<usize>,
    history_draft: String,
    completion: Option<Completion>,
    pub error: String,
}

struct Completion {
    before: String,
    after: String,
    matches: Vec<String>,
    index: usize,
    result: (String, usize),
}

impl EnhancedChat {
    pub fn remember_sent(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let history = self.sent.entry(self.audience.clone()).or_default();
        history.retain(|previous| previous != text);
        history.push_front(text.into());
        history.truncate(20);
    }

    pub fn sent_history(&self) -> Vec<String> {
        self.sent
            .get(&self.audience)
            .map(|history| history.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn completion_prefix(text: &str, caret: usize) -> &str {
        let before = text.get(..caret).unwrap_or_default();
        before
            .rsplit_once(char::is_whitespace)
            .map_or(before, |(_, word)| word)
    }

    pub fn complete(
        &mut self,
        text: &str,
        caret: usize,
        candidates: &[String],
        backwards: bool,
    ) -> Option<(String, usize)> {
        if let Some(previous) = self
            .completion
            .as_mut()
            .filter(|c| c.result.0 == text && c.result.1 == caret)
        {
            let count = previous.matches.len();
            previous.index = (previous.index + if backwards { count - 1 } else { 1 }) % count;
        } else {
            let prefix = Self::completion_prefix(text, caret);
            if prefix.is_empty() {
                return None;
            }
            let matches: Vec<_> = candidates
                .iter()
                .filter(|name| name.to_lowercase().starts_with(&prefix.to_lowercase()))
                .cloned()
                .collect();
            if matches.is_empty() {
                return None;
            }
            self.completion = Some(Completion {
                before: text.get(..caret.checked_sub(prefix.len())?)?.into(),
                after: text.get(caret..)?.into(),
                index: if backwards { matches.len() - 1 } else { 0 },
                matches,
                result: (String::new(), 0),
            });
        }
        let completion = self.completion.as_mut()?;
        let name = &completion.matches[completion.index];
        completion.result = (
            format!("{}{name}{}", completion.before, completion.after),
            completion.before.len() + name.len(),
        );
        Some(completion.result.clone())
    }

    pub fn draft(&self) -> String {
        self.drafts.get(&self.audience).cloned().unwrap_or_default()
    }

    pub fn save_draft(&mut self, text: &str) {
        self.drafts.insert(self.audience.clone(), text.into());
    }

    pub fn switch_audience(&mut self, audience: ChatAudience, text: &str) -> String {
        self.save_draft(text);
        self.audience = audience;
        self.history_index = None;
        self.draft()
    }

    pub fn reset_history(&mut self) {
        self.history_index = None;
    }

    pub fn browse_history(&mut self, older: bool, text: &str, history: &[String]) -> String {
        if !older && self.history_index.is_none() {
            return text.into();
        }
        if older {
            if self.history_index.is_none() {
                self.history_draft = text.into();
            }
            self.history_index = match (self.history_index, history.is_empty()) {
                (_, true) => None,
                (None, false) => Some(0),
                (Some(index), false) => Some((index + 1).min(history.len() - 1)),
            };
        } else {
            self.history_index = self.history_index.and_then(|index| index.checked_sub(1));
        }
        self.history_index
            .and_then(|index| history.get(index))
            .cloned()
            .unwrap_or_else(|| self.history_draft.clone())
    }

    pub fn push(&mut self, mut message: ChatMessage) {
        message.id = self.next_id;
        self.next_id += 1;
        if self.anchor.is_some() && (self.show_logs || message.channel != ChatChannel::Log) {
            self.unread += 1;
        }
        self.messages.push_back(message);
        while self.messages.len() > 1000
            || self
                .messages
                .iter()
                .map(|m| m.text.len() + m.sender.len())
                .sum::<usize>()
                > 256_000
        {
            self.messages.pop_front();
        }
        if let (Some(anchor), Some(first)) = (self.anchor, self.messages.front()) {
            self.anchor = Some(anchor.max(first.id));
        }
    }

    pub fn toggle_logs(&mut self) {
        self.show_logs = !self.show_logs;
        self.jump_to_latest();
    }

    pub fn visible_messages(&self) -> Vec<&ChatMessage> {
        self.messages
            .iter()
            .filter(|message| self.show_logs || message.channel != ChatChannel::Log)
            .filter(|message| self.anchor.is_none_or(|id| message.id <= id))
            .collect()
    }

    pub fn matching_messages(&self) -> Vec<&ChatMessage> {
        self.messages
            .iter()
            .filter(|message| self.show_logs || message.channel != ChatChannel::Log)
            .collect()
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.jump_to_latest();
    }

    pub fn scroll(&mut self, older: bool) {
        let counts = self
            .matching_messages()
            .iter()
            .map(|m| (m.id, 1))
            .collect::<Vec<_>>();
        self.scroll_wrapped(older, &counts);
    }

    pub fn line_offset(&self) -> usize {
        self.line_offset
    }

    pub fn scroll_wrapped(&mut self, older: bool, counts: &[(u64, usize)]) {
        if counts.is_empty() {
            return;
        }
        let index = self
            .anchor
            .and_then(|id| counts.iter().position(|(candidate, _)| *candidate >= id))
            .unwrap_or(counts.len() - 1);
        let (id, rows) = counts[index];
        self.line_offset = self.line_offset.min(rows.saturating_sub(1));
        self.anchor = Some(id);
        if older {
            if self.line_offset + 1 < rows {
                self.line_offset += 1;
            } else if index > 0 {
                self.anchor = Some(counts[index - 1].0);
                self.line_offset = 0;
            }
        } else if self.line_offset > 0 {
            self.line_offset -= 1;
        } else if index + 1 < counts.len() {
            self.anchor = Some(counts[index + 1].0);
            self.line_offset = counts[index + 1].1.saturating_sub(1);
        } else {
            self.jump_to_latest();
        }
        if self.anchor == counts.last().map(|(id, _)| *id) && self.line_offset == 0 && !older {
            self.jump_to_latest();
        }
    }

    pub fn unread(&self) -> usize {
        self.unread
    }

    pub fn jump_to_latest(&mut self) {
        self.anchor = None;
        self.line_offset = 0;
        self.unread = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_history_keeps_private_message_bodies_with_their_recipient() {
        let mut chat = EnhancedChat::default();
        chat.remember_sent("public message");
        chat.switch_audience(ChatAudience::Private(7), "");
        chat.remember_sent("private message");
        assert_eq!(chat.sent_history(), vec!["private message"]);
        chat.switch_audience(ChatAudience::Everyone, "");
        assert_eq!(chat.sent_history(), vec!["public message"]);
    }

    #[test]
    fn down_arrow_without_history_navigation_preserves_the_current_text() {
        let mut chat = EnhancedChat::default();
        assert_eq!(
            chat.browse_history(false, "still writing", &["sent".into()]),
            "still writing"
        );
    }

    #[test]
    fn completion_cycles_matches_without_losing_text_after_the_caret() {
        let mut chat = EnhancedChat::default();
        let names = vec!["Ada".into(), "Adam".into(), "Bea".into()];
        let first = chat.complete("Ad, help", 2, &names, false).unwrap();
        assert_eq!(first, ("Ada, help".into(), 3));
        let second = chat.complete(&first.0, first.1, &names, false).unwrap();
        assert_eq!(second, ("Adam, help".into(), 4));
        let previous = chat.complete(&second.0, second.1, &names, true).unwrap();
        assert_eq!(previous, first);
        assert_eq!(
            chat.complete("Be", 2, &names, false),
            Some(("Bea".into(), 3))
        );
    }

    #[test]
    fn wrapped_messages_can_be_read_one_line_at_a_time() {
        let mut chat = EnhancedChat::default();
        chat.push(ChatMessage::conversation(
            "Ada",
            ChatChannel::Everyone,
            "long message",
        ));
        let id = chat.visible_messages()[0].id;
        chat.scroll_wrapped(true, &[(id, 8)]);
        chat.scroll_wrapped(true, &[(id, 8)]);
        assert_eq!(chat.line_offset(), 2);
        chat.push(ChatMessage::conversation(
            "Ada",
            ChatChannel::Everyone,
            "new",
        ));
        assert_eq!(chat.line_offset(), 2);
        assert_eq!(chat.visible_messages().len(), 1);
        chat.jump_to_latest();
        assert_eq!(chat.line_offset(), 0);
    }

    #[test]
    fn conversation_filter_excludes_log_noise_and_bounds_retained_history() {
        let mut chat = EnhancedChat::default();
        chat.push(ChatMessage::conversation(
            "Ada",
            ChatChannel::Everyone,
            "hello",
        ));
        chat.scroll(true);
        chat.push(ChatMessage::conversation(
            "",
            ChatChannel::Log,
            "script output",
        ));
        assert_eq!(chat.unread(), 0);
        chat.jump_to_latest();
        assert_eq!(chat.visible_messages().len(), 1);
        chat.toggle_logs();
        assert_eq!(chat.visible_messages().len(), 2);
        for _ in 0..1100 {
            chat.push(ChatMessage::conversation(
                "Ada",
                ChatChannel::Everyone,
                "more",
            ));
        }
        assert!(chat.visible_messages().len() <= 1000);
    }

    #[test]
    fn history_and_audience_switching_restore_each_unsent_draft() {
        let mut chat = EnhancedChat::default();
        chat.save_draft("work in progress");
        assert_eq!(
            chat.browse_history(true, "work in progress", &["sent".into()]),
            "sent"
        );
        assert_eq!(
            chat.browse_history(false, "sent", &["sent".into()]),
            "work in progress"
        );
        chat.switch_audience(ChatAudience::Allies, "work in progress");
        chat.save_draft("secret plan");
        assert_eq!(
            chat.switch_audience(ChatAudience::Everyone, "secret plan"),
            "work in progress"
        );
        assert_eq!(
            chat.switch_audience(ChatAudience::Allies, "work in progress"),
            "secret plan"
        );
    }

    #[test]
    fn incoming_messages_preserve_the_reading_anchor_and_count_unread() {
        let mut chat = EnhancedChat::default();
        for text in ["first", "second", "third"] {
            chat.push(ChatMessage::conversation(
                "Ada",
                ChatChannel::Everyone,
                text,
            ));
        }
        chat.scroll(true);
        let anchor = chat.visible_messages().last().unwrap().id;
        chat.push(ChatMessage::conversation(
            "Ada",
            ChatChannel::Everyone,
            "fourth",
        ));
        assert_eq!(chat.visible_messages().last().unwrap().id, anchor);
        assert_eq!(chat.unread(), 1);
        chat.jump_to_latest();
        assert_eq!(chat.visible_messages().last().unwrap().text, "fourth");
        assert_eq!(chat.unread(), 0);
    }
}
