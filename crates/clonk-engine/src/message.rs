use crate::{ObjectId, ObjectMenuFrameDecoration, SpeechFallback, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    Append {
        spec: MessageSpec,
        no_duplicates: bool,
    },
    PendingSpeech(SpeechFallback),
}

#[derive(Debug, Clone)]
struct Message {
    id: u64,
    order: u64,
    kind: MessageKind,
    text: String,
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

#[derive(Debug, Clone)]
struct PendingSpeechMessage {
    order: u64,
    spec: MessageSpec,
    elapsed_ticks: u32,
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
        let normalized_text = self.lines.join("|");
        PersistedMessage {
            snapshot: self.to_snapshot(),
            remaining: self.remaining,
            raw_text: (self.text != normalized_text).then(|| self.text.clone()),
        }
    }
}

#[derive(Debug, Default)]
pub struct MessageManager {
    next_id: u64,
    next_order: u64,
    messages: Vec<Message>,
    pending_speech: HashMap<u64, PendingSpeechMessage>,
}

impl MessageManager {
    /// `C4GameMessageList::UpdateDef` (`C4GameMessage.cpp:340-345`), which
    /// `C4Game::ReloadDef` runs as its **last** act after either arm
    /// (`C4Game.cpp:2364`) — success or failure alike.
    ///
    /// Each live message's frame decoration re-resolves against the reloaded
    /// definition (`C4GameMessage::UpdateDef` -> `pFrameDeco->UpdateGfx()`,
    /// `:233-244`), and a decoration the definition no longer supplies is
    /// **deleted** rather than left drawing from a definition that is gone.
    /// Decorations sourced from other definitions are untouched.
    ///
    /// Returns how many decorations were dropped.
    pub fn update_def(&mut self, definition: &str, still_supplied: bool) -> usize {
        if still_supplied {
            return 0;
        }
        let mut dropped = 0;
        for message in &mut self.messages {
            let sourced_here = message
                .frame_decoration
                .as_ref()
                .is_some_and(|decoration| decoration.source_definition == definition);
            if sourced_here {
                message.frame_decoration = None;
                dropped += 1;
            }
        }
        dropped
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_command(&mut self, command: MessageCommand) {
        match command {
            MessageCommand::Add(spec) => {
                self.add_message(spec);
            }
            MessageCommand::Append {
                spec,
                no_duplicates,
            } => {
                self.append_message(spec, no_duplicates);
            }
            MessageCommand::PendingSpeech(fallback) => {
                self.reserve_speech_fallback(fallback);
            }
        }
    }

    pub fn add_message(&mut self, spec: MessageSpec) {
        let order = self.allocate_order();
        if !spec.allows_multiple() {
            self.remove_conflicting(&spec, None);
            self.pending_speech.retain(|_, pending| {
                !message_fields_conflict_with_spec(
                    pending.spec.target,
                    pending.spec.player,
                    pending.spec.flags,
                    &spec,
                )
            });
        }
        self.insert_message(spec, order, 0);
    }

    /// `C4GameMessageList::Append` reuses the first matching message and
    /// retains all of its presentation metadata. With `fNoDuplicates`, each
    /// `|`-delimited text start is prefix-compared before the new line is
    /// appended (C4GameMessage.cpp:73-83,315-329;
    /// C4Strings.cpp:110-118).
    pub fn append_message(&mut self, mut spec: MessageSpec, no_duplicates: bool) {
        let mut appended_text = clonk_script::c4_string_bytes(&spec.text);
        if let Some(terminator) = appended_text.iter().position(|&byte| byte == 0) {
            appended_text.truncate(terminator);
            spec.text = clonk_script::c4_string_from_bytes(&appended_text);
        }
        let matching_index = self.messages.iter().position(|message| {
            (spec.kind == MessageKind::Target && message.target == spec.target)
                || (matches!(spec.kind, MessageKind::Global | MessageKind::GlobalPlayer)
                    && message.player == spec.player)
        });
        let matching_index =
            matching_index.filter(|&index| self.messages[index].target == spec.target);
        let Some(message) = matching_index.and_then(|index| self.messages.get_mut(index)) else {
            // C4GameMessageList::Append delegates to New for a missing
            // message. New treats an empty string as a successful delete/no-op
            // request rather than inserting an empty message.
            if spec.text.is_empty() {
                return;
            }
            self.add_message(spec);
            return;
        };

        let existing_bytes = clonk_script::c4_string_bytes(&message.text);
        let existing_text = existing_bytes
            .split(|&byte| byte == 0)
            .next()
            .unwrap_or_default();
        let duplicate = no_duplicates
            && std::iter::once(0)
                .chain(
                    existing_text
                        .iter()
                        .enumerate()
                        .filter(|&(_, &byte)| byte == b'|')
                        .map(|(index, _)| index.saturating_add(1)),
                )
                .take_while(|&index| index < existing_text.len())
                .any(|index| existing_text[index..].starts_with(&appended_text));
        if duplicate {
            return;
        }

        message.text.push('|');
        message.text.push_str(&spec.text);
        message.lines.extend(
            spec.text
                .split('|')
                .map(|line| line.trim_end_matches('\r').to_string()),
        );
        let added_delay = i32::try_from(clonk_script::c4_string_byte_len(&spec.text))
            .unwrap_or(i32::MAX)
            .saturating_mul(DELAY_FACTOR);
        message.remaining = message.remaining.saturating_add(added_delay);
    }

    pub(crate) fn resolve_speech_fallback(&mut self, fallback: SpeechFallback, rejected: bool) {
        let fallback_id = fallback.id();
        let Some(pending) = self.pending_speech.remove(&fallback_id) else {
            return;
        };
        debug_assert_eq!(pending.spec, fallback.into_message());
        if !rejected {
            return;
        }
        if !pending.spec.allows_multiple() {
            self.remove_conflicting(&pending.spec, Some(pending.order));
        }
        self.insert_message(pending.spec, pending.order, pending.elapsed_ticks);
    }

    fn reserve_speech_fallback(&mut self, fallback: SpeechFallback) {
        let id = fallback.id();
        let order = self.allocate_order();
        let replaced = self.pending_speech.insert(
            id,
            PendingSpeechMessage {
                order,
                spec: fallback.into_message(),
                elapsed_ticks: 0,
            },
        );
        assert!(replaced.is_none(), "speech fallback id must be unique");
    }

    fn insert_message(&mut self, spec: MessageSpec, order: u64, elapsed_ticks: u32) {
        let mut text = if (spec.flags & FLAG_DROP_SPEECH) != 0 {
            spec.text.split('$').next().unwrap_or("").to_string()
        } else {
            spec.text.clone()
        };

        if text.is_empty() {
            return;
        }

        // StdStrBuf::getLength() reports bytes and includes `|` separators
        // when C4GameMessage::Init derives Delay (C4GameMessage.cpp:37-52).
        let delay_length =
            i32::try_from(clonk_script::c4_string_byte_len(&text)).unwrap_or(i32::MAX);
        let mut permanent = false;
        if let Some(stripped) = text.strip_prefix('@') {
            permanent = true;
            text = stripped.to_string();
        }

        let lines: Vec<String> = text
            .split('|')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();
        let mut remaining = if permanent {
            -1
        } else {
            delay_length.saturating_mul(DELAY_FACTOR).max(MIN_DELAY)
        };
        if remaining == 0 {
            remaining = MIN_DELAY;
        }
        if remaining > 0 {
            remaining = remaining.saturating_sub(i32::try_from(elapsed_ticks).unwrap_or(i32::MAX));
            if remaining <= 0 {
                return;
            }
        }

        let id = self.allocate_id();
        let message = Message {
            id,
            order,
            kind: spec.kind,
            text,
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
        };
        let index = self
            .messages
            .partition_point(|existing| existing.order < order);
        self.messages.insert(index, message);
    }

    #[allow(dead_code)]
    pub fn clear_for_object(&mut self, id: ObjectId) {
        self.messages.retain(|message| message.target != Some(id));
        self.pending_speech
            .retain(|_, pending| pending.spec.target != Some(id));
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
        self.pending_speech.retain(|_, pending| {
            if let Some(target) = pending.spec.target {
                if !existing_objects.contains(&target) {
                    return false;
                }
            }
            pending.elapsed_ticks = pending.elapsed_ticks.saturating_add(1);
            true
        });
    }

    pub fn snapshot(&self) -> Vec<MessageSnapshot> {
        self.messages
            .iter()
            .map(Message::to_snapshot)
            .collect::<Vec<_>>()
    }

    pub fn line_contains(&self, needle: &str) -> bool {
        self.messages
            .iter()
            .any(|message| message.lines.iter().any(|line| line.contains(needle)))
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
            .enumerate()
            .map(|(order, persisted)| {
                let PersistedMessage {
                    snapshot,
                    remaining,
                    raw_text,
                } = persisted;
                let text = raw_text.unwrap_or_else(|| snapshot.lines.join("|"));
                Message {
                    id: snapshot.id,
                    order: u64::try_from(order).unwrap_or(u64::MAX),
                    kind: snapshot.kind,
                    text,
                    lines: snapshot.lines,
                    target: snapshot.target,
                    player: snapshot.player,
                    offset: snapshot.offset,
                    color: snapshot.color,
                    flags: snapshot.flags,
                    width: snapshot.width,
                    decoration: snapshot.decoration,
                    frame_decoration: snapshot.frame_decoration,
                    portrait: snapshot.portrait,
                    remaining,
                }
            })
            .collect();
        self.pending_speech.clear();
        self.recalculate_next_id();
        self.recalculate_next_order();
    }

    fn remove_conflicting(&mut self, spec: &MessageSpec, before_order: Option<u64>) {
        self.messages.retain(|message| {
            if before_order.is_some_and(|order| message.order >= order) {
                return true;
            }
            !message_fields_conflict_with_spec(message.target, message.player, message.flags, spec)
        });
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn allocate_order(&mut self) -> u64 {
        let order = self.next_order;
        self.next_order = self.next_order.wrapping_add(1).max(1);
        order
    }

    fn recalculate_next_id(&mut self) {
        self.next_id = self
            .messages
            .iter()
            .map(|message| message.id.wrapping_add(1))
            .max()
            .unwrap_or(1);
    }

    fn recalculate_next_order(&mut self) {
        self.next_order = self
            .messages
            .iter()
            .map(|message| message.order.wrapping_add(1))
            .max()
            .unwrap_or(1);
    }
}

fn message_fields_conflict_with_spec(
    target: Option<ObjectId>,
    player: Option<i32>,
    flags: u32,
    spec: &MessageSpec,
) -> bool {
    match spec.kind {
        MessageKind::Target | MessageKind::TargetPlayer => spec
            .target
            .is_some_and(|spec_target| target == Some(spec_target)),
        MessageKind::Global | MessageKind::GlobalPlayer => {
            player == spec.player && (flags & POSITIONING_FLAGS) == (spec.flags & POSITIONING_FLAGS)
        }
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
    fn append_reuses_target_message_and_prefix_deduplicates_at_line_starts() {
        // C4GameMessageList::Append retains the first same-target record.
        // fNoDuplicates checks SEqual2 at every `|`-delimited suffix, while
        // distinct text extends that record and its delay
        // (C4GameMessage.cpp:73-83,315-329; C4Strings.cpp:110-118).
        let target = ObjectId::new(7);
        let original = MessageSpec {
            kind: MessageKind::Target,
            text: "Site|needs|1x Wood".to_owned(),
            target: Some(target),
            player: Some(4),
            offset: Vector2::new(-1, -1),
            color: 0xff12_3456,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        };
        let mut messages = MessageManager::new();
        messages.add_message(original.clone());
        let before = messages.persisted().into_iter().next().expect("message");
        assert_eq!(
            before.remaining, 36,
            "C4GameMessage::Init counts the two `|` bytes in its delay"
        );

        let mut duplicate = original.clone();
        duplicate.text = "needs|1x Wood".to_owned();
        duplicate.player = None;
        duplicate.offset = Vector2::ZERO;
        messages.append_message(duplicate, true);
        assert_eq!(messages.persisted()[0].snapshot, before.snapshot);
        assert_eq!(messages.persisted()[0].remaining, before.remaining);

        let mut distinct = original;
        distinct.text = "No wood nearby".to_owned();
        distinct.player = None;
        distinct.offset = Vector2::ZERO;
        messages.append_message(distinct.clone(), true);
        let persisted = messages.persisted();
        assert_eq!(persisted.len(), 1);
        let appended = &persisted[0];
        assert_eq!(appended.snapshot.id, before.snapshot.id);
        assert_eq!(appended.snapshot.player, Some(4));
        assert_eq!(appended.snapshot.offset, Vector2::new(-1, -1));
        assert_eq!(
            appended.snapshot.lines,
            ["Site", "needs", "1x Wood", "No wood nearby"]
        );
        assert_eq!(
            appended.remaining,
            before.remaining + i32::try_from(distinct.text.len()).unwrap() * DELAY_FACTOR
        );
    }

    #[test]
    fn append_empty_text_does_not_insert_a_message() {
        // C4GameMessageList::Append delegates to New when there is no
        // matching record, and New returns after its empty-text clear path
        // without creating a record (C4GameMessage.cpp:290-305,315-329).
        let mut messages = MessageManager::new();
        messages.append_message(tutorial_message(""), false);
        assert!(messages.snapshot().is_empty());
    }

    #[test]
    fn append_uses_legacy_bytes_and_preserves_duplicate_text_across_restore() {
        // C4GameMessage stores StdStrBuf bytes verbatim. Init/Append use native
        // byte lengths, and SEqual2 includes a carriage return in its prefix
        // comparison (C4GameMessage.cpp:37-83; C4Strings.cpp:110-118).
        let target = ObjectId::new(8);
        let raw_text = clonk_script::c4_string_from_bytes(b"123456789\xe9\r|tail");
        let duplicate_text = clonk_script::c4_string_from_bytes(b"123456789\xe9\r");
        let mut messages = MessageManager::new();
        messages.add_message(MessageSpec {
            kind: MessageKind::Target,
            text: raw_text,
            target: Some(target),
            player: None,
            offset: Vector2::ZERO,
            color: 0xffff_ffff,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
        let persisted = messages.persisted();
        assert_eq!(
            persisted[0].remaining, 32,
            "the 16 legacy bytes, not Rust's projected UTF-8 length, set Delay"
        );

        messages.restore(persisted);
        let before = messages.persisted().remove(0);
        messages.append_message(
            MessageSpec {
                kind: MessageKind::Target,
                text: duplicate_text,
                target: Some(target),
                player: None,
                offset: Vector2::ZERO,
                color: 0xffff_ffff,
                flags: 0,
                width: None,
                decoration: None,
                frame_decoration: None,
                portrait: None,
            },
            true,
        );
        assert_eq!(messages.persisted()[0].snapshot, before.snapshot);
        assert_eq!(
            messages.persisted()[0].remaining,
            before.remaining,
            "a byte-identical prefix remains a duplicate after save/restore"
        );

        let nul_duplicate = clonk_script::c4_string_from_bytes(b"tail\0ignored");
        messages.append_message(
            MessageSpec {
                kind: MessageKind::Target,
                text: nul_duplicate,
                target: Some(target),
                player: None,
                offset: Vector2::ZERO,
                color: 0xffff_ffff,
                flags: 0,
                width: None,
                decoration: None,
                frame_decoration: None,
                portrait: None,
            },
            true,
        );
        assert_eq!(
            messages.persisted()[0].remaining,
            before.remaining,
            "SEqual2 sees only bytes before the incoming C-string terminator"
        );

        let nul_distinct = clonk_script::c4_string_from_bytes(b"new\0ignored");
        messages.append_message(
            MessageSpec {
                kind: MessageKind::Target,
                text: nul_distinct,
                target: Some(target),
                player: None,
                offset: Vector2::ZERO,
                color: 0xffff_ffff,
                flags: 0,
                width: None,
                decoration: None,
                frame_decoration: None,
                portrait: None,
            },
            false,
        );
        assert_eq!(
            messages.persisted()[0].snapshot.lines.last().unwrap(),
            "new"
        );
        assert_eq!(
            messages.persisted()[0].remaining,
            before.remaining + 3 * DELAY_FACTOR,
            "Text.Append and SLen stop at the incoming C-string terminator"
        );

        let distinct_text = clonk_script::c4_string_from_bytes(b"\xe9");
        messages.append_message(
            MessageSpec {
                kind: MessageKind::Target,
                text: distinct_text,
                target: Some(target),
                player: None,
                offset: Vector2::ZERO,
                color: 0xffff_ffff,
                flags: 0,
                width: None,
                decoration: None,
                frame_decoration: None,
                portrait: None,
            },
            false,
        );
        assert_eq!(
            messages.persisted()[0].remaining,
            before.remaining + 4 * DELAY_FACTOR,
            "Append adds two frames for one legacy byte"
        );
    }

    #[test]
    fn deferred_speech_resolution_preserves_original_message_order() {
        let mut messages = MessageManager::new();
        messages.add_message(tutorial_message("@older"));
        let rejected = SpeechFallback::new(1, tutorial_message("speech fallback"));
        messages.apply_command(MessageCommand::PendingSpeech(rejected.clone()));
        messages.add_message(tutorial_message("later"));
        messages.resolve_speech_fallback(rejected, true);
        assert_eq!(messages.snapshot()[0].lines, ["later"]);

        let mut messages = MessageManager::new();
        messages.add_message(tutorial_message("@older"));
        let played = SpeechFallback::new(2, tutorial_message("unused fallback"));
        messages.apply_command(MessageCommand::PendingSpeech(played.clone()));
        messages.resolve_speech_fallback(played, false);
        assert_eq!(messages.snapshot()[0].lines, ["older"]);

        let mut messages = MessageManager::new();
        messages.add_message(tutorial_message("@older"));
        let rejected = SpeechFallback::new(3, tutorial_message("speech fallback"));
        messages.apply_command(MessageCommand::PendingSpeech(rejected.clone()));
        let mut multiple = tutorial_message("later multiple");
        multiple.flags |= FLAG_MULTIPLE;
        messages.add_message(multiple);
        messages.resolve_speech_fallback(rejected, true);
        let snapshot = messages.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].lines, ["speech fallback"]);
        assert_eq!(snapshot[1].lines, ["later multiple"]);
    }

    #[test]
    fn deferred_speech_fallback_expires_while_admission_is_pending() {
        let mut messages = MessageManager::new();
        let rejected = SpeechFallback::new(1, tutorial_message("x"));
        messages.apply_command(MessageCommand::PendingSpeech(rejected.clone()));
        for _ in 0..=MIN_DELAY {
            messages.tick(&HashSet::new());
        }

        messages.resolve_speech_fallback(rejected, true);

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
