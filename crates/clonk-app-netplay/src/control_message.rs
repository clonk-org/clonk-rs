use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Process-local state behind `C4Client::TryAllowSound` and `C4Client::muted`.
///
/// The native cooldown lives in the single global configuration object, so it
/// is shared by every sender. Mute state is client-local and starts from the
/// same configured default for each client.
#[derive(Debug)]
pub struct ControlMessageState {
    sound_cooldown: Duration,
    last_sound_command: Option<Instant>,
    default_muted: bool,
    muted_clients: HashMap<i32, bool>,
    user_attention_pending: bool,
}

impl ControlMessageState {
    pub fn new(sound_cooldown: Duration, default_muted: bool) -> Self {
        Self {
            sound_cooldown,
            last_sound_command: None,
            default_muted,
            muted_clients: HashMap::new(),
            user_attention_pending: false,
        }
    }

    /// `C4Cooldown::TryReset`: the first call succeeds, and a successful call
    /// consumes the one global cooldown before callers inspect mute state.
    pub fn try_allow_sound_at(&mut self, now: Instant) -> bool {
        let elapsed = self.last_sound_command.is_none_or(|last| {
            now.checked_duration_since(last)
                .is_some_and(|elapsed| elapsed >= self.sound_cooldown)
        });
        if elapsed {
            self.last_sound_command = Some(now);
        }
        elapsed
    }

    pub fn is_muted(&self, client_id: i32) -> bool {
        self.muted_clients
            .get(&client_id)
            .copied()
            .unwrap_or(self.default_muted)
    }

    pub fn set_muted(&mut self, client_id: i32, muted: bool) {
        self.muted_clients.insert(client_id, muted);
    }

    pub fn remove_client(&mut self, client_id: i32) {
        self.muted_clients.remove(&client_id);
    }

    pub fn clear_clients(&mut self) {
        self.muted_clients.clear();
    }

    pub fn request_user_attention(&mut self) {
        self.user_attention_pending = true;
    }

    pub fn take_user_attention_request(&mut self) -> bool {
        std::mem::take(&mut self.user_attention_pending)
    }
}

fn capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - (b'a' - b'A'),
        0xe4 => 0xc4, // ä -> Ä in the legacy single-byte charset
        0xf6 => 0xd6, // ö -> Ö
        0xfc => 0xdc, // ü -> Ü
        _ => byte,
    }
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'~' | b'+' | b'-')
}

/// The first-match, case-insensitive nick check used by
/// `C4ControlMessage::Execute`. Identifier characters on either side prevent
/// an alert; native does not continue searching after such a rejected match.
pub fn mentions_nick(message: &[u8], nick: &[u8]) -> bool {
    if nick.is_empty() || nick.len() > message.len() {
        return false;
    }
    // SSearchNoCase is deliberately not a backtracking substring search: a
    // mismatch resets the matched prefix to zero and advances to the next
    // message byte. Preserve that old overlap behavior as well as returning
    // only the first match.
    let mut matched = 0;
    let mut end = None;
    for (index, &byte) in message.iter().enumerate() {
        if capital(byte) == capital(nick[matched]) {
            matched += 1;
        } else {
            matched = 0;
        }
        if matched >= nick.len() {
            end = Some(index + 1);
            break;
        }
    }
    let Some(end) = end else {
        return false;
    };
    let start = end - nick.len();
    let before_is_identifier = start > 0 && identifier(message[start - 1]);
    let after_is_identifier = message.get(end).copied().is_some_and(identifier);
    !before_is_identifier && !after_is_identifier
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_cooldown_is_global_and_consumed_before_mute_policy() {
        let start = Instant::now();
        let mut state = ControlMessageState::new(Duration::from_secs(5), false);
        state.set_muted(7, true);

        assert!(state.try_allow_sound_at(start));
        assert!(state.is_muted(7));
        assert!(!state.try_allow_sound_at(start + Duration::from_secs(4)));
        assert!(state.try_allow_sound_at(start + Duration::from_secs(5)));

        state.set_muted(7, false);
        assert!(!state.is_muted(7));
        assert!(!state.try_allow_sound_at(start + Duration::from_secs(6)));
    }

    #[test]
    fn zero_sound_cooldown_allows_every_sender() {
        let now = Instant::now();
        let mut state = ControlMessageState::new(Duration::ZERO, false);
        assert!(state.try_allow_sound_at(now));
        assert!(state.try_allow_sound_at(now));
    }

    #[test]
    fn nick_alert_uses_legacy_identifier_boundaries_and_first_match() {
        assert!(mentions_nick(b"hi aLi!", b"Ali"));
        for embedded in [
            b"Malice".as_slice(),
            b"Ali-ce",
            b"Ali+ce",
            b"Ali_ce",
            b"Ali~ce",
        ] {
            assert!(!mentions_nick(embedded, b"Ali"), "{embedded:?}");
        }
        assert!(!mentions_nick(b"Malice, Ali!", b"Ali"));
        assert!(mentions_nick(b"\xe4!", b"\xc4"));
        assert!(!mentions_nick(b"aaab!", b"aab"));
    }

    #[test]
    fn attention_request_is_coalesced_until_consumed() {
        let mut state = ControlMessageState::new(Duration::ZERO, false);
        state.request_user_attention();
        state.request_user_attention();
        assert!(state.take_user_attention_request());
        assert!(!state.take_user_attention_request());
    }

    #[test]
    fn mute_policy_resets_with_native_client_lifetime() {
        let mut state = ControlMessageState::new(Duration::ZERO, false);
        state.set_muted(7, true);
        state.set_muted(8, true);
        state.remove_client(7);
        assert!(!state.is_muted(7));
        assert!(state.is_muted(8));
        state.clear_clients();
        assert!(!state.is_muted(8));
    }
}
