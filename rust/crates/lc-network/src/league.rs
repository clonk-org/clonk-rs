use std::collections::HashMap;

use lc_engine::LegacyCString;
use sha1::{Digest, Sha1};
use thiserror::Error;

const CHECKSUM_PLACEHOLDER: &[u8; 5] = b"-----";
const CHECKSUM_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const CHECKSUM_TARGET: u32 = 0x7a69;
const CHECKSUM_MASK: u32 = 0xf0ff;

/// The authentication-only fields of `C4LeagueRequestHead`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueAuthRequestHead {
    pub account: LegacyCString,
    pub password: LegacyCString,
    pub new_account: LegacyCString,
    pub new_password: LegacyCString,
}

/// The session identifiers carried by a host-side player authentication check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueJoinRequestHead {
    pub csid: LegacyCString,
    pub auid: LegacyCString,
}

/// Serializes the `[Request]` part of a player authentication request.
pub fn encode_league_auth_request_head(request: &LeagueAuthRequestHead) -> Vec<u8> {
    let mut output = b"[Request]\r\nAction=Auth\r\nChecksum=".to_vec();
    output.extend_from_slice(CHECKSUM_PLACEHOLDER);
    output.extend_from_slice(b"\r\n");
    push_raw_field(&mut output, b"Account", &request.account);
    push_raw_field(&mut output, b"Password", &request.password);
    push_raw_field(&mut output, b"NewAccount", &request.new_account);
    push_raw_field(&mut output, b"NewPassword", &request.new_password);
    output
}

/// Builds the complete `Auth` query from an exact serialized player section.
pub fn encode_league_auth_request(
    request: &LeagueAuthRequestHead,
    player_info_section: &[u8],
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueChecksumError> {
    finish_player_request(
        encode_league_auth_request_head(request),
        player_info_section,
        checksum_start,
    )
}

/// Serializes the `[Request]` part of a player authentication check.
pub fn encode_league_join_request_head(request: &LeagueJoinRequestHead) -> Vec<u8> {
    let mut output = b"[Request]\r\nAction=Join\r\n".to_vec();
    push_raw_field(&mut output, b"CSID", &request.csid);
    push_raw_field(&mut output, b"AUID", &request.auid);
    output.extend_from_slice(b"Checksum=");
    output.extend_from_slice(CHECKSUM_PLACEHOLDER);
    output.extend_from_slice(b"\r\n");
    output
}

/// Builds the complete `Join` query from an exact serialized player section.
pub fn encode_league_join_request(
    request: &LeagueJoinRequestHead,
    player_info_section: &[u8],
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueChecksumError> {
    finish_player_request(
        encode_league_join_request_head(request),
        player_info_section,
        checksum_start,
    )
}

fn finish_player_request(
    mut request: Vec<u8>,
    player_info_section: &[u8],
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueChecksumError> {
    if !player_info_section.is_empty() {
        request.extend_from_slice(b"\r\n");
        request.extend_from_slice(player_info_section);
    }
    solve_league_checksum(&mut request, checksum_start)?;
    Ok(request)
}

/// Failure to locate or solve the checksum placeholder of a league request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeagueChecksumError {
    #[error("league request has no five-byte checksum placeholder")]
    MissingPlaceholder,
    #[error("league checksum search exhausted every C++ candidate")]
    SearchExhausted,
}

/// Common response fields returned by the league server for `Auth`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueAuthResponse {
    pub status: LegacyCString,
    pub csid: LegacyCString,
    pub message: LegacyCString,
    pub account: LegacyCString,
    pub auid: LegacyCString,
    pub fbid: LegacyCString,
}

impl LeagueAuthResponse {
    pub fn is_success(&self) -> bool {
        self.status.as_bytes().eq_ignore_ascii_case(b"Success")
    }

    pub fn is_register(&self) -> bool {
        self.status.as_bytes().eq_ignore_ascii_case(b"Register")
    }
}

/// Decodes the common `[Response]` fields of an authentication reply.
pub fn decode_league_auth_response(input: &[u8]) -> LeagueAuthResponse {
    LeagueAuthResponse {
        status: response_identifier(input, b"Status"),
        csid: response_identifier(input, b"CSID"),
        message: response_raw(input, b"Message"),
        account: response_raw(input, b"Account"),
        auid: response_raw(input, b"AUID"),
        fbid: response_raw(input, b"FBID"),
    }
}

pub const MAX_LEAGUES: usize = 10;

/// Player league data returned by a host-side `Join` authentication check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueJoinResponse {
    pub head: LeagueAuthResponse,
    pub leagues: [LegacyCString; MAX_LEAGUES],
    pub scores: [i32; MAX_LEAGUES],
    pub ranks: [i32; MAX_LEAGUES],
    pub rank_symbols: [i32; MAX_LEAGUES],
    pub progress_data: [LegacyCString; MAX_LEAGUES],
    pub clan_tag: LegacyCString,
}

/// Decodes an authentication-check (`Action=Join`) response.
pub fn decode_league_join_response(input: &[u8]) -> LeagueJoinResponse {
    LeagueJoinResponse {
        head: decode_league_auth_response(input),
        leagues: parse_escaped_array(response_field(input, b"League").unwrap_or_default()),
        scores: parse_i32_array(response_field(input, b"Score").unwrap_or_default()),
        ranks: parse_i32_array(response_field(input, b"Rank").unwrap_or_default()),
        rank_symbols: parse_i32_array(response_field(input, b"RankSymbol").unwrap_or_default()),
        progress_data: parse_escaped_array(
            response_field(input, b"ProgressData").unwrap_or_default(),
        ),
        clan_tag: response_raw(input, b"ClanTag"),
    }
}

/// Replaces the first `-----` marker with the C++ league proof-of-work value.
///
/// `start` is the already-combined value called `iStart` by C++; keeping it an
/// argument lets the eventual client inject the two legacy `rand()` results.
pub fn solve_league_checksum(data: &mut [u8], start: u32) -> Result<[u8; 5], LeagueChecksumError> {
    let replace = data
        .windows(CHECKSUM_PLACEHOLDER.len())
        .position(|window| window == CHECKSUM_PLACEHOLDER)
        .ok_or(LeagueChecksumError::MissingPlaceholder)?;
    let mut hasher = Sha1::new();
    for iteration in 0_u32..(1_u32 << 30) {
        let value = iteration ^ start;
        let mut candidate = [0_u8; 5];
        for (index, byte) in candidate.iter_mut().enumerate() {
            *byte = CHECKSUM_ALPHABET[((value >> (index * 5)) & 63) as usize];
        }
        data[replace..replace + candidate.len()].copy_from_slice(&candidate);
        hasher.update(&*data);
        let digest = hasher.finalize_reset();
        let first_word = u32::from_ne_bytes([digest[0], digest[1], digest[2], digest[3]]);
        if ((first_word ^ CHECKSUM_TARGET) & CHECKSUM_MASK) == 0 {
            return Ok(candidate);
        }
    }
    Err(LeagueChecksumError::SearchExhausted)
}

fn push_raw_field(output: &mut Vec<u8>, name: &[u8], value: &LegacyCString) {
    if value.is_empty() {
        return;
    }
    output.extend_from_slice(name);
    output.push(b'=');
    output.extend_from_slice(value.as_bytes());
    output.extend_from_slice(b"\r\n");
}

fn response_identifier(input: &[u8], key: &[u8]) -> LegacyCString {
    let value = response_field(input, key).unwrap_or_default();
    let end = value
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_' && *byte != b'-')
        .unwrap_or(value.len());
    legacy_string(&value[..end])
}

fn response_raw(input: &[u8], key: &[u8]) -> LegacyCString {
    legacy_string(response_field(input, key).unwrap_or_default())
}

fn response_field<'a>(input: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let input = input.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut in_response = false;
    let mut saw_response = false;
    for line in input.split(|byte| *byte == b'\r' || *byte == b'\n') {
        let line = trim_horizontal_start(line);
        if line.starts_with(b"[") {
            if saw_response {
                in_response = false;
                continue;
            }
            in_response = line == b"[Response]";
            saw_response = in_response;
            continue;
        }
        if !in_response {
            continue;
        }
        let Some(equal) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        if trim_horizontal_end(&line[..equal]) == key {
            return Some(trim_horizontal_start(&line[equal + 1..]));
        }
    }
    None
}

fn trim_horizontal_start(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.first(), Some(b' ' | b'\t')) {
        bytes = &bytes[1..];
    }
    bytes
}

fn trim_horizontal_end(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.last(), Some(b' ' | b'\t')) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn legacy_string(bytes: &[u8]) -> LegacyCString {
    LegacyCString::from_bytes(bytes.to_vec())
        .expect("response bytes were truncated before their first NUL")
}

fn parse_escaped_array(raw: &[u8]) -> [LegacyCString; MAX_LEAGUES] {
    let mut values: [LegacyCString; MAX_LEAGUES] =
        std::array::from_fn(|_| LegacyCString::default());
    let mut remaining = trim_horizontal_start(raw);
    for value in &mut values {
        if remaining.is_empty() {
            break;
        }
        let (decoded, rest, quoted) = parse_escaped_value(remaining);
        *value = legacy_string(&decoded);
        if !quoted {
            break;
        }
        remaining = trim_horizontal_start(rest);
        if !remaining.starts_with(b",") {
            break;
        }
        remaining = trim_horizontal_start(&remaining[1..]);
    }
    values
}

fn parse_escaped_value(raw: &[u8]) -> (Vec<u8>, &[u8], bool) {
    let Some(mut remaining) = raw.strip_prefix(b"\"") else {
        return (raw.to_vec(), &[], false);
    };
    let mut output = Vec::new();
    while let Some((&byte, rest)) = remaining.split_first() {
        remaining = rest;
        if byte == b'"' {
            return (output, remaining, true);
        }
        if byte != b'\\' {
            output.push(byte);
            continue;
        }
        let Some((&escape, rest)) = remaining.split_first() else {
            break;
        };
        remaining = rest;
        match escape {
            b'a' => output.push(b'\x07'),
            b'b' => output.push(b'\x08'),
            b'f' => output.push(b'\x0c'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(b'\x0b'),
            b'\'' | b'"' | b'\\' | b'?' => output.push(escape),
            digit @ b'0'..=b'7' => {
                let mut code = u32::from(digit - b'0');
                while let Some((&next @ b'0'..=b'7', rest)) = remaining.split_first() {
                    code = code.wrapping_mul(8).wrapping_add(u32::from(next - b'0'));
                    remaining = rest;
                }
                output.push(code as u8);
            }
            other => output.push(other),
        }
    }
    (output, remaining, true)
}

fn parse_i32_array(raw: &[u8]) -> [i32; MAX_LEAGUES] {
    let mut values = [0; MAX_LEAGUES];
    let mut remaining = raw;
    for value in &mut values {
        remaining = trim_horizontal_start(remaining);
        let end = remaining
            .iter()
            .position(|byte| *byte == b',')
            .unwrap_or(remaining.len());
        let token = trim_horizontal_end(&remaining[..end]);
        *value = std::str::from_utf8(token)
            .ok()
            .and_then(|token| token.parse().ok())
            .unwrap_or_default();
        if end == remaining.len() {
            break;
        }
        remaining = &remaining[end + 1..];
    }
    values
}

/// Tracks league feedback identifiers (FBIDs) for authenticated accounts.
///
/// The classic runtime keeps a linked list that is synchronised with the list of
/// players reported to the league backend. Consumers look up FBIDs by account
/// name when constructing disconnect reports or when restoring cached
/// authentication state.  This registry mirrors the semantics of
/// `C4LeagueFBIDList`: inserting a new FBID replaces any previous mapping for
/// the account and removal is a no-op if the entry does not exist.
#[derive(Debug, Clone, Default)]
pub struct LeagueFbidRegistry {
    entries: HashMap<String, String>,
}

impl LeagueFbidRegistry {
    /// Returns an empty registry.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Clears every stored FBID.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Associates `account` with `fbid`, replacing any previous value.
    pub fn insert(&mut self, account: impl Into<String>, fbid: impl Into<String>) {
        self.entries.insert(account.into(), fbid.into());
    }

    /// Removes the FBID associated with `account`.
    ///
    /// Returns `true` if an entry was present.
    pub fn remove(&mut self, account: &str) -> bool {
        self.entries.remove(account).is_some()
    }

    /// Looks up the FBID registered for `account`.
    pub fn get(&self, account: &str) -> Option<&str> {
        self.entries.get(account).map(|value| value.as_str())
    }

    /// Returns the number of tracked accounts.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reports whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_league_auth_response, decode_league_join_response, encode_league_auth_request,
        encode_league_auth_request_head, encode_league_join_request_head, solve_league_checksum,
        LeagueAuthRequestHead, LeagueFbidRegistry, LeagueJoinRequestHead,
    };
    use lc_engine::LegacyCString;

    fn legacy(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).expect("test value contains no NUL")
    }

    #[test]
    fn auth_request_head_matches_cpp_field_order_and_raw_string_bytes() {
        // C4LeagueRequestHead::CompileFunc writes Action/Checksum first, then
        // the four RCT_All authentication strings in this order
        // (src/C4League.cpp:31-56). RCT_All is emitted byte-for-byte rather
        // than as a quoted escaped string (src/StdCompiler.cpp:362-375).
        let request = LeagueAuthRequestHead {
            account: legacy(b"A\x80 \\\""),
            password: legacy(b"p=ass"),
            new_account: legacy(b"new user"),
            new_password: LegacyCString::default(),
        };

        assert_eq!(
            encode_league_auth_request_head(&request),
            b"[Request]\r\n\
Action=Auth\r\n\
Checksum=-----\r\n\
Account=A\x80 \\\"\r\n\
Password=p=ass\r\n\
NewAccount=new user\r\n"
        );
    }

    #[test]
    fn join_request_head_matches_cpp_field_order_and_raw_ids() {
        // AuthCheck constructs C4LeagueRequestHead(Join, CSID, AUID), sets the
        // five-byte placeholder and serializes it (src/C4League.cpp:451-464).
        // The writer does not transform RCT_IdtfAllowEmpty bytes
        // (src/StdCompiler.cpp:362-375).
        let request = LeagueJoinRequestHead {
            csid: legacy(b"session-\x80"),
            auid: legacy(b"auth_id"),
        };

        assert_eq!(
            encode_league_join_request_head(&request),
            b"[Request]\r\n\
Action=Join\r\n\
CSID=session-\x80\r\n\
AUID=auth_id\r\n\
Checksum=-----\r\n"
        );
    }

    #[test]
    fn checksum_search_matches_cpp_five_byte_fixture() {
        // C4LeagueClient::ModifyForChecksum enumerates five Base64Tbl bytes
        // from low-to-high five-bit chunks of (iteration ^ start), SHA-1s the
        // entire request, and tests the native first word against
        // checksum/mask 0x7a69/0xf0ff (src/C4League.cpp:513-545).
        let mut request = b"[Request]\r\nAction=Auth\r\nChecksum=-----\r\n".to_vec();

        assert_eq!(
            solve_league_checksum(&mut request, 0).expect("fixture has a solution"),
            *b"c0BAA"
        );
        assert_eq!(request, b"[Request]\r\nAction=Auth\r\nChecksum=c0BAA\r\n");
    }

    #[test]
    fn auth_response_preserves_cpp_raw_fields_and_identifier_status() {
        // The response head reads Status/CSID as RCT_IdtfAllowEmpty and the
        // message/authentication values as RCT_All (src/C4League.cpp:102-112).
        // INI read skips leading horizontal whitespace, preserves the rest of
        // an RCT_All line, and stops an identifier at punctuation
        // (src/StdCompiler.cpp:897-1001).
        let response = decode_league_auth_response(
            b"[Response]\r\n\
Status=sUcCeSs!ignored\r\n\
CSID=session-1.trailing\r\n\
Message= \tWelcome \x80  \r\n\
Account=Player, One\r\n\
AUID=auth=value\r\n\
FBID=feedback id\r\n",
        );

        assert_eq!(response.status.as_bytes(), b"sUcCeSs");
        assert_eq!(response.csid.as_bytes(), b"session-1");
        assert_eq!(response.message.as_bytes(), b"Welcome \x80  ");
        assert_eq!(response.account.as_bytes(), b"Player, One");
        assert_eq!(response.auid.as_bytes(), b"auth=value");
        assert_eq!(response.fbid.as_bytes(), b"feedback id");
        assert!(response.is_success());
        assert!(!response.is_register());
    }

    #[test]
    fn join_response_decodes_cpp_arrays_and_escaped_bytes() {
        // AuthCheck replies add ten-element comma-separated League/Score/Rank/
        // RankSymbol/ProgressData arrays plus raw ClanTag
        // (src/C4League.cpp:178-192). StdCompiler's escaped reader preserves
        // arbitrary bytes and permits commas inside quotes
        // (src/StdCompiler.cpp:903-1060).
        let response = decode_league_join_response(
            b"[Response]\r\n\
Status=Success\r\n\
Account=Player\r\n\
League=\"Cup, One\",\"L\\200\"\r\n\
Score=7,-2\r\n\
Rank=3,4\r\n\
RankSymbol=5,6\r\n\
ProgressData=\"done,yes\",\"\\377\\61\"\r\n\
ClanTag= \tTAG \x80 \r\n",
        );

        assert!(response.head.is_success());
        assert_eq!(response.leagues[0].as_bytes(), b"Cup, One");
        assert_eq!(response.leagues[1].as_bytes(), b"L\x80");
        assert_eq!(&response.scores[..3], &[7, -2, 0]);
        assert_eq!(&response.ranks[..3], &[3, 4, 0]);
        assert_eq!(&response.rank_symbols[..3], &[5, 6, 0]);
        assert_eq!(response.progress_data[0].as_bytes(), b"done,yes");
        assert_eq!(response.progress_data[1].as_bytes(), b"\xff1");
        assert_eq!(response.clan_tag.as_bytes(), b"TAG \x80 ");
    }

    #[test]
    fn complete_auth_request_hashes_the_exact_injected_player_section() {
        // Auth inserts the already-serialized C4PlayerInfo after Request and
        // then runs ModifyForChecksum over the complete byte buffer
        // (src/C4League.cpp:401-420; StdCompiler.cpp:473-485).
        let request = LeagueAuthRequestHead {
            account: legacy(b"A\x80 \\\""),
            password: legacy(b"p=ass"),
            new_account: legacy(b"new user"),
            new_password: LegacyCString::default(),
        };

        assert_eq!(
            encode_league_auth_request(&request, b"[PlrInfo]\r\nName=\"P\\200\"\r\n", 0x1234_5678,)
                .expect("fixture checksum solves"),
            b"[Request]\r\n\
Action=Auth\r\n\
Checksum=JMUoj\r\n\
Account=A\x80 \\\"\r\n\
Password=p=ass\r\n\
NewAccount=new user\r\n\
\r\n\
[PlrInfo]\r\n\
Name=\"P\\200\"\r\n"
        );
    }

    #[test]
    fn insert_and_lookup_round_trip() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("Alice"), Some("FBID-123"));
        assert_eq!(registry.get("Bob"), Some("FBID-456"));
        assert!(registry.get("Eve").is_none());
    }

    #[test]
    fn replacing_existing_account_overwrites_value() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Alice", "FBID-999");

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("Alice"), Some("FBID-999"));
    }

    #[test]
    fn removing_unknown_account_is_a_noop() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        assert!(!registry.remove("Bob"));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("Alice"), Some("FBID-123"));
    }

    #[test]
    fn removal_drops_entry() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");

        assert!(registry.remove("Alice"));
        assert_eq!(registry.get("Alice"), None);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");
        registry.clear();

        assert!(registry.is_empty());
        assert_eq!(registry.get("Alice"), None);
        assert_eq!(registry.get("Bob"), None);
    }
}
