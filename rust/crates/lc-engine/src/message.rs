use crate::{ObjectId, ObjectMenuFrameDecoration, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MIN_DELAY: i32 = 20;
const DELAY_FACTOR: i32 = 2;

pub const FLAG_NO_BREAK: u32 = 1 << 0;
pub const FLAG_BOTTOM: u32 = 1 << 1;
pub const FLAG_MULTIPLE: u32 = 1 << 2;
pub const FLAG_TOP: u32 = 1 << 3;
pub const FLAG_LEFT: u32 = 1 << 4;
pub const FLAG_RIGHT: u32 = 1 << 5;
pub const FLAG_HCENTER: u32 = 1 << 6;
pub const FLAG_VCENTER: u32 = 1 << 7;
pub const FLAG_DROP_SPEECH: u32 = 1 << 8;
pub const FLAG_WIDTH_REL: u32 = 1 << 9;
pub const FLAG_X_REL: u32 = 1 << 10;
pub const FLAG_Y_REL: u32 = 1 << 11;
pub const FLAG_ALIGN_LEFT: u32 = 1 << 12;
pub const FLAG_ALIGN_CENTER: u32 = 1 << 13;
pub const FLAG_ALIGN_RIGHT: u32 = 1 << 14;

pub const POSITIONING_FLAGS: u32 =
    FLAG_BOTTOM | FLAG_TOP | FLAG_LEFT | FLAG_RIGHT | FLAG_HCENTER | FLAG_VCENTER;
pub const HORIZONTAL_POSITION_FLAGS: u32 = FLAG_LEFT | FLAG_HCENTER | FLAG_RIGHT;
pub const VERTICAL_POSITION_FLAGS: u32 = FLAG_TOP | FLAG_VCENTER | FLAG_BOTTOM;
pub const ALIGNMENT_FLAGS: u32 = FLAG_ALIGN_LEFT | FLAG_ALIGN_CENTER | FLAG_ALIGN_RIGHT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Global,
    GlobalPlayer,
    Target,
    TargetPlayer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSnapshot {
    pub id: u64,
    pub kind: MessageKind,
    pub lines: Vec<String>,
    pub target: Option<ObjectId>,
    pub player: Option<i32>,
    pub offset: Vector2,
    pub color: u32,
    pub flags: u32,
    pub width: Option<i32>,
    pub decoration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_decoration: Option<ObjectMenuFrameDecoration>,
    pub portrait: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    pub snapshot: MessageSnapshot,
    pub remaining: i32,
}

#[derive(Debug, Clone)]
pub struct MessageSpec {
    pub kind: MessageKind,
    pub text: String,
    pub target: Option<ObjectId>,
    pub player: Option<i32>,
    pub offset: Vector2,
    pub color: u32,
    pub flags: u32,
    pub width: Option<i32>,
    pub decoration: Option<String>,
    pub frame_decoration: Option<ObjectMenuFrameDecoration>,
    pub portrait: Option<String>,
}

impl MessageSpec {
    fn allows_multiple(&self) -> bool {
        (self.flags & FLAG_MULTIPLE) != 0
    }
}

#[derive(Debug, Clone)]
pub enum MessageCommand {
    Add(MessageSpec),
}

#[derive(Debug, Clone)]
struct Message {
    id: u64,
    kind: MessageKind,
    lines: Vec<String>,
    target: Option<ObjectId>,
    player: Option<i32>,
    offset: Vector2,
    color: u32,
    flags: u32,
    width: Option<i32>,
    decoration: Option<String>,
    frame_decoration: Option<ObjectMenuFrameDecoration>,
    portrait: Option<String>,
    remaining: i32,
}

impl Message {
    fn to_snapshot(&self) -> MessageSnapshot {
        MessageSnapshot {
            id: self.id,
            kind: self.kind,
            lines: self.lines.clone(),
            target: self.target,
            player: self.player,
            offset: self.offset,
            color: self.color,
            flags: self.flags,
            width: self.width,
            decoration: self.decoration.clone(),
            frame_decoration: self.frame_decoration.clone(),
            portrait: self.portrait.clone(),
        }
    }

    fn to_persisted(&self) -> PersistedMessage {
        PersistedMessage {
            snapshot: self.to_snapshot(),
            remaining: self.remaining,
        }
    }
}

#[derive(Debug, Default)]
pub struct MessageManager {
    next_id: u64,
    messages: Vec<Message>,
}

impl MessageManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_command(&mut self, command: MessageCommand) {
        match command {
            MessageCommand::Add(spec) => {
                self.add_message(spec);
            }
        }
    }

    pub fn add_message(&mut self, spec: MessageSpec) {
        let mut text = if (spec.flags & FLAG_DROP_SPEECH) != 0 {
            spec.text.split('$').next().unwrap_or("").to_string()
        } else {
            spec.text.clone()
        };

        if !spec.allows_multiple() {
            self.remove_conflicting(&spec);
        }

        if text.is_empty() {
            return;
        }

        let mut permanent = false;
        if let Some(stripped) = text.strip_prefix('@') {
            permanent = true;
            text = stripped.to_string();
        }

        let lines: Vec<String> = text
            .split('|')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();
        let total_chars: usize = lines.iter().map(|line| line.chars().count()).sum();
        let mut remaining = if permanent {
            -1
        } else {
            (total_chars as i32 * DELAY_FACTOR).max(MIN_DELAY)
        };
        if remaining == 0 {
            remaining = MIN_DELAY;
        }

        let id = self.allocate_id();
        self.messages.push(Message {
            id,
            kind: spec.kind,
            lines,
            target: spec.target,
            player: spec.player,
            offset: spec.offset,
            color: spec.color,
            flags: spec.flags,
            width: spec.width,
            decoration: spec.decoration,
            frame_decoration: spec.frame_decoration,
            portrait: spec.portrait,
            remaining,
        });
    }

    #[allow(dead_code)]
    pub fn clear_for_object(&mut self, id: ObjectId) {
        self.messages.retain(|message| message.target != Some(id));
    }

    pub fn tick(&mut self, existing_objects: &HashSet<ObjectId>) {
        self.messages.retain_mut(|message| {
            if let Some(target) = message.target {
                if !existing_objects.contains(&target) {
                    return false;
                }
            }
            if message.remaining > 0 {
                message.remaining -= 1;
            }
            if message.remaining == 0 {
                return false;
            }
            true
        });
    }

    pub fn snapshot(&self) -> Vec<MessageSnapshot> {
        self.messages
            .iter()
            .map(Message::to_snapshot)
            .collect::<Vec<_>>()
    }

    pub fn persisted(&self) -> Vec<PersistedMessage> {
        self.messages
            .iter()
            .map(Message::to_persisted)
            .collect::<Vec<_>>()
    }

    pub fn restore(&mut self, messages: Vec<PersistedMessage>) {
        self.messages = messages
            .into_iter()
            .map(|persisted| Message {
                id: persisted.snapshot.id,
                kind: persisted.snapshot.kind,
                lines: persisted.snapshot.lines,
                target: persisted.snapshot.target,
                player: persisted.snapshot.player,
                offset: persisted.snapshot.offset,
                color: persisted.snapshot.color,
                flags: persisted.snapshot.flags,
                width: persisted.snapshot.width,
                decoration: persisted.snapshot.decoration,
                frame_decoration: persisted.snapshot.frame_decoration,
                portrait: persisted.snapshot.portrait,
                remaining: persisted.remaining,
            })
            .collect();
        self.recalculate_next_id();
    }

    fn remove_conflicting(&mut self, spec: &MessageSpec) {
        match spec.kind {
            MessageKind::Target | MessageKind::TargetPlayer => {
                if let Some(target) = spec.target {
                    self.messages
                        .retain(|message| message.target != Some(target));
                }
            }
            MessageKind::Global | MessageKind::GlobalPlayer => {
                let positioning = spec.flags & POSITIONING_FLAGS;
                self.messages.retain(|message| {
                    if message.player != spec.player {
                        return true;
                    }
                    let existing_positioning = message.flags & POSITIONING_FLAGS;
                    existing_positioning != positioning
                });
            }
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn recalculate_next_id(&mut self) {
        self.next_id = self
            .messages
            .iter()
            .map(|message| message.id.wrapping_add(1))
            .max()
            .unwrap_or(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tutorial_message(text: &str) -> MessageSpec {
        MessageSpec {
            kind: MessageKind::GlobalPlayer,
            text: text.to_string(),
            target: None,
            player: Some(0),
            offset: Vector2::new(10, -30),
            color: 0xffff_ffff,
            flags: FLAG_BOTTOM | FLAG_LEFT | FLAG_X_REL | FLAG_WIDTH_REL,
            width: Some(35),
            decoration: Some("DECO".to_string()),
            frame_decoration: None,
            portrait: Some("Portrait:SCLK::0000ff::1".to_string()),
        }
    }

    #[test]
    fn empty_non_multiple_message_clears_conflicting_permanent_message() {
        // C4GameMessageList::New removes same-player/same-position messages
        // before treating empty text as a successful clear-only operation
        // (C4GameMessage.cpp:290-305).
        let mut messages = MessageManager::new();
        messages.add_message(tutorial_message("@Build the elevator."));
        assert_eq!(messages.snapshot().len(), 1);

        messages.add_message(tutorial_message(""));

        assert!(messages.snapshot().is_empty());
    }

    #[test]
    fn frame_decoration_survives_snapshot_persistence_and_legacy_deserialization() {
        let frame_decoration = ObjectMenuFrameDecoration {
            source_definition: "DECO".to_string(),
            background_color: 0x8032_3232,
            border_top: 0,
            border_left: 0,
            border_right: 0,
            border_bottom: 0,
            top: None,
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: None,
        };
        let mut spec = tutorial_message("@Welcome to the world of Clonk.");
        spec.frame_decoration = Some(frame_decoration.clone());
        let mut messages = MessageManager::new();
        messages.add_message(spec);

        assert_eq!(
            messages.snapshot()[0].frame_decoration,
            Some(frame_decoration.clone())
        );

        let persisted = serde_json::to_vec(&messages.persisted()).expect("messages serialize");
        let persisted = serde_json::from_slice(&persisted).expect("messages deserialize");
        let mut restored = MessageManager::new();
        restored.restore(persisted);
        assert_eq!(
            restored.snapshot()[0].frame_decoration,
            Some(frame_decoration)
        );

        let mut legacy = serde_json::to_value(&restored.snapshot()[0]).expect("snapshot encodes");
        legacy
            .as_object_mut()
            .expect("snapshot is an object")
            .remove("frame_decoration");
        let legacy: MessageSnapshot =
            serde_json::from_value(legacy).expect("legacy snapshot remains readable");
        assert!(legacy.frame_decoration.is_none());
    }
}
