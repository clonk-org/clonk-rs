use std::collections::HashMap;
use std::time::Duration;

use clonk_engine::{
    ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS, CLIENT_PLAYER_INFO_FLAG_INITIAL,
    CLIENT_PLAYER_INFO_FLAG_UPDATED, NETWORK_RESOURCE_TYPE_NULL, PLAYER_INFO_FLAG_ATTRIBUTES_FIXED,
    PLAYER_INFO_FLAG_DISCONNECTED, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
    PLAYER_INFO_FLAG_IN_SCENARIO_FILE, PLAYER_INFO_FLAG_JOINED,
    PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK, PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
    PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_FLAG_SAVEGAME_JOIN, PLAYER_INFO_FLAG_VOTED_OUT,
    PLAYER_INFO_FLAG_WON, PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
};
use sha1::{Digest, Sha1};
use thiserror::Error;

use crate::advertise::encode_host_game_reference_response;
use crate::host_game_reference::{HostGameReference, HostGameReferenceError};
use crate::join_player_registry::{ClientPlayerInfosSnapshot, PlayerInfoListSnapshot};
use crate::league_round_results_packet::{LeagueRoundPlayerStatus, LeagueRoundResultsPlayer};
use crate::name_validation::{validate_name_allow_empty, validate_name_no_empty};

const CHECKSUM_PLACEHOLDER: &[u8; 5] = b"-----";
const CHECKSUM_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const CHECKSUM_TARGET: u32 = 0x7a69;
const CHECKSUM_MASK: u32 = 0xf0ff;
const MAX_PLAYER_INFO_COUNT: usize = 5_000;
const NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE: u32 = 100 * 1024;
const PLAYER_INFO_FLAG_JOIN_ISSUED: u16 = 1 << 4;

/// `C4NetMinLeagueUpdateInterval`, installed by `InvalidateReference`.
pub const LEAGUE_MIN_UPDATE_INTERVAL_SECONDS: i64 = 10;

/// `C4HTTPQueryTimeout` used for every league query.
pub const LEAGUE_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Release `C4ENGINENAME/C4VERSION` sent by the pinned C++ oracle.
pub const LEAGUE_HTTP_USER_AGENT: &str = concat!("LegacyClonk/", clonk_core::engine_version_str!());

/// Language settings copied from `Config.General` for a league query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueHttpTransportConfig {
    /// `Config.General.LanguageCharset`, before C++ code-page normalization.
    pub language_charset: String,
    /// `Config.General.LanguageEx`, preserved byte-for-byte as a header value.
    pub language_sequence: String,
}

impl LeagueHttpTransportConfig {
    fn charset_code_name(&self) -> &'static str {
        match self.language_charset.to_ascii_uppercase().as_str() {
            "SHIFTJIS" => "CP932",
            "HANGUL" => "CP949",
            "JOHAB" => "CP1361",
            "CHINESEBIG5" => "CP950",
            "GREEK" => "CP1253",
            "TURKISH" => "CP1254",
            "VIETNAMESE" => "CP1258",
            "HEBREW" => "CP1255",
            "ARABIC" => "CP1256",
            "BALTIC" => "CP1257",
            "RUSSIAN" => "CP1251",
            "THAI" => "CP874",
            "EASTEUROPE" => "CP1250",
            "UTF-8" => "UTF-8",
            _ => "CP1252",
        }
    }
}

/// Raw HTTP failure while exchanging an already-serialized league request.
#[derive(Debug, Error)]
pub enum LeagueHttpTransportError {
    #[error("league HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
}

/// Injectable HTTP client for C++-format league request and response bytes.
#[derive(Debug, Clone)]
pub struct LeagueHttpPostTransport {
    client: reqwest::Client,
}

impl LeagueHttpPostTransport {
    /// Builds the C++ transport policy: gzip, redirects and a per-query
    /// 20-second timeout. HTTP error statuses fail like `CURLOPT_FAILONERROR`;
    /// redirects and cookies persist for the lifetime of this client
    /// (`src/C4HTTPClient.cpp:183-229`).
    pub fn cpp_default() -> Result<Self, LeagueHttpTransportError> {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .gzip(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                attempt.follow()
            }))
            .build()?;
        Ok(Self { client })
    }

    /// Uses a caller-supplied client while retaining exact request headers,
    /// body bytes, timeout and status handling. This is the injection seam for
    /// callers that own proxy/TLS policy or deterministic tests.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// POSTs an encoded league request and returns the response body without
    /// interpreting its legacy bytes.
    pub async fn post(
        &self,
        endpoint: &str,
        body: &[u8],
        config: &LeagueHttpTransportConfig,
    ) -> Result<Vec<u8>, LeagueHttpTransportError> {
        let charset = config.charset_code_name();
        let response = self
            .client
            .post(endpoint)
            .timeout(LEAGUE_HTTP_TIMEOUT)
            .header(reqwest::header::USER_AGENT, LEAGUE_HTTP_USER_AGENT)
            .header(reqwest::header::ACCEPT_CHARSET, charset)
            .header(reqwest::header::ACCEPT_ENCODING, "gzip")
            .header(reqwest::header::ACCEPT_LANGUAGE, &config.language_sequence)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("text/plain; encoding={charset}"),
            )
            .body(body.to_vec())
            .send()
            .await?
            .error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

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

/// A `C4PlayerInfo` value that the C++ INI decompiler cannot represent safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeaguePlayerInfoEncodeError {
    #[error("player {player_id} has unsupported type {player_type}")]
    InvalidPlayerType { player_id: i32, player_type: u8 },
    #[error("player {0} has HasResource without a resource core")]
    MissingPlayerResource(i32),
    #[error("player {0} carries a resource core without HasResource")]
    UnexpectedPlayerResource(i32),
    #[error("resource type {0} has no C++ text representation")]
    InvalidResourceType(u8),
    #[error("loadable resource {0} has zero chunk size")]
    ZeroResourceChunkSize(i32),
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

/// Record metadata appended to an `Action=End` request.
///
/// The C++ writer emits the SHA whenever the record name is nonempty. Keeping
/// the two values paired prevents its undefined null-SHA call shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueEndRecord {
    pub name: LegacyCString,
    pub sha1: [u8; 20],
}

/// Failure while building a Start/Update/End request around an exact host
/// reference.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeagueReferenceRequestEncodeError {
    #[error(transparent)]
    Reference(#[from] HostGameReferenceError),
    #[error(transparent)]
    Checksum(#[from] LeagueChecksumError),
    #[error("league host session has no CSID")]
    MissingCsid,
}

/// Builds the exact `C4LA_Start` request and solves its checksum over both
/// sibling sections.
pub fn encode_league_start_request(
    reference: &HostGameReference,
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
    finish_reference_request(
        b"[Request]\r\nAction=Start\r\nChecksum=-----\r\n".to_vec(),
        reference,
        checksum_start,
    )
}

/// Builds the exact `C4LA_Update` request for a registered host session.
pub fn encode_league_update_request(
    csid: &LegacyCString,
    reference: &HostGameReference,
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
    if csid.is_empty() {
        return Err(LeagueReferenceRequestEncodeError::MissingCsid);
    }
    let mut request = b"[Request]\r\nAction=Update\r\nCSID=".to_vec();
    request.extend_from_slice(csid.as_bytes());
    request.extend_from_slice(b"\r\nChecksum=-----\r\n");
    finish_reference_request(request, reference, checksum_start)
}

/// Builds the exact `C4LA_End` request. Empty record names are treated like
/// the native no-record path and omit both record fields.
pub fn encode_league_end_request(
    csid: &LegacyCString,
    reference: &HostGameReference,
    record: Option<&LeagueEndRecord>,
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
    if csid.is_empty() {
        return Err(LeagueReferenceRequestEncodeError::MissingCsid);
    }
    let mut request = b"[Request]\r\nAction=End\r\nCSID=".to_vec();
    request.extend_from_slice(csid.as_bytes());
    request.extend_from_slice(b"\r\nChecksum=-----\r\n");
    if let Some(record) = record.filter(|record| !record.name.is_empty()) {
        request.extend_from_slice(b"RecordName=");
        request.extend_from_slice(record.name.as_bytes());
        request.extend_from_slice(b"\r\nRecordSHA=");
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in record.sha1 {
            request.push(HEX[usize::from(byte >> 4)]);
            request.push(HEX[usize::from(byte & 0x0f)]);
        }
        request.extend_from_slice(b"\r\n");
    }
    finish_reference_request(request, reference, checksum_start)
}

/// Reason sent with `C4LA_ReportDisconnect`.
///
/// The native INI writer elides `Unknown`, its default value, and uses the
/// two identifiers below for the actionable reasons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LeagueDisconnectReason {
    #[default]
    Unknown,
    ConnectionFailed,
    Desync,
}

impl LeagueDisconnectReason {
    fn cpp_identifier(self) -> Option<&'static [u8]> {
        match self {
            Self::Unknown => None,
            Self::ConnectionFailed => Some(b"ConnectionFailed"),
            Self::Desync => Some(b"Desync"),
        }
    }
}

/// Builds the exact `C4LA_ReportDisconnect` request for one client's players.
///
/// Unlike Update and End, the native client permits an empty CSID here: a
/// joined client may need to report the host before it has received a session
/// identifier. Every joined, non-removed player is emitted in packet order.
/// The FBID is optional and looked up by the player's byte-preserving league
/// account.
pub fn encode_league_report_disconnect_request(
    csid: &LegacyCString,
    reason: LeagueDisconnectReason,
    player_infos: &ClientPlayerInfosSnapshot,
    fbids: &LeagueFbidRegistry,
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueChecksumError> {
    let mut request = b"[Request]\r\nAction=ReportDisconnect\r\n".to_vec();
    push_raw_field(&mut request, b"CSID", csid);
    request.extend_from_slice(b"Checksum=-----\r\n");
    if let Some(reason) = reason.cpp_identifier() {
        request.extend_from_slice(b"Reason=");
        request.extend_from_slice(reason);
        request.extend_from_slice(b"\r\n");
    }

    let mut emitted_player_infos = false;
    for player in &player_infos.players {
        if player.flags & PLAYER_INFO_FLAG_JOINED == 0
            || player.flags & PLAYER_INFO_FLAG_REMOVED != 0
        {
            continue;
        }
        if !emitted_player_infos {
            request.extend_from_slice(b"\r\n[PlayerInfos]\r\n");
            emitted_player_infos = true;
        }
        request.extend_from_slice(b"\r\n  [Player]\r\n  ID=");
        request.extend_from_slice(player.id.to_string().as_bytes());
        request.extend_from_slice(b"\r\n");
        if let Some(fbid) = fbids.get(&player.league_account) {
            request.extend_from_slice(b"  FBID=");
            request.extend_from_slice(fbid.as_bytes());
            request.extend_from_slice(b"\r\n");
        }
    }
    solve_league_checksum(&mut request, checksum_start)?;
    Ok(request)
}

/// Decodes the common response returned for `ReportDisconnect`.
pub fn decode_league_report_disconnect_response(input: &[u8]) -> LeagueAuthResponse {
    decode_league_auth_response(input)
}

fn finish_reference_request(
    mut request: Vec<u8>,
    reference: &HostGameReference,
    checksum_start: u32,
) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
    let reference = encode_host_game_reference_response(reference)?;
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(&reference);
    solve_league_checksum(&mut request, checksum_start)?;
    Ok(request)
}

/// Pure wall-clock state for the host's league reference heartbeat.
///
/// Times are integer seconds because the native scheduler compares
/// `time(nullptr)` values. A Start does not arm a delay: the first sec1 tick is
/// immediately due. Only a successfully dispatched Update advances the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueHeartbeat {
    configured_period_secs: i64,
    last_update_started_at: Option<i64>,
    delay_secs: i64,
}

impl LeagueHeartbeat {
    pub fn new(configured_period_secs: i64) -> Self {
        Self {
            configured_period_secs,
            last_update_started_at: None,
            delay_secs: configured_period_secs,
        }
    }

    /// Mirrors `!iLastLeagueUpdate || now > last + delay`, including its
    /// strict boundary.
    pub fn is_due(&self, now: i64) -> bool {
        self.last_update_started_at
            .is_none_or(|last| now > last.saturating_add(self.delay_secs))
    }

    /// Records successful query dispatch, before any HTTP or reply result is
    /// known, and restores the configured period.
    pub fn update_dispatched(&mut self, now: i64) {
        self.last_update_started_at = Some(now);
        self.delay_secs = self.configured_period_secs;
    }

    /// Shortens the interval relative to the previous dispatch timestamp.
    pub fn invalidate_reference(&mut self) {
        self.delay_secs = LEAGUE_MIN_UPDATE_INTERVAL_SECONDS;
    }

    pub fn last_update_started_at(&self) -> Option<i64> {
        self.last_update_started_at
    }

    pub fn delay_secs(&self) -> i64 {
        self.delay_secs
    }

    pub fn configured_period_secs(&self) -> i64 {
        self.configured_period_secs
    }

    pub fn set_configured_period_secs(&mut self, configured_period_secs: i64) {
        self.configured_period_secs = configured_period_secs;
    }
}

/// Saved host-side registration identity. Start response validation is the
/// only operation that installs a CSID; Update and End always reuse it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueHostSession {
    csid: LegacyCString,
}

impl LeagueHostSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn csid(&self) -> Option<&LegacyCString> {
        (!self.csid.is_empty()).then_some(&self.csid)
    }

    pub fn clear(&mut self) {
        self.csid = LegacyCString::default();
    }

    pub fn accept_start_response(
        &mut self,
        input: &[u8],
    ) -> Result<LeagueStartResponse, LeagueResponseDecodeError> {
        let response = decode_league_start_response(input)?;
        self.csid = response.head.csid.clone();
        Ok(response)
    }

    pub fn encode_update_request(
        &self,
        reference: &HostGameReference,
        checksum_start: u32,
    ) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
        encode_league_update_request(&self.csid, reference, checksum_start)
    }

    pub fn encode_end_request(
        &self,
        reference: &HostGameReference,
        record: Option<&LeagueEndRecord>,
        checksum_start: u32,
    ) -> Result<Vec<u8>, LeagueReferenceRequestEncodeError> {
        encode_league_end_request(&self.csid, reference, record, checksum_start)
    }
}

/// Serializes the exact `[PlrInfo]` sibling inserted into league Auth/Join
/// requests by `C4LeagueClient` (`src/C4League.cpp:401-420,451-466`).
pub fn encode_league_player_info_section(
    player: &ControlPlayerInfoEntry,
) -> Result<Vec<u8>, LeaguePlayerInfoEncodeError> {
    validate_league_player_info(player)?;
    let mut output = String::from("[PlrInfo]\r\n");
    crate::host_game_reference::append_player_info_fields(&mut output, player, 0);
    Ok(output
        .chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect())
}

fn validate_league_player_info(
    player: &ControlPlayerInfoEntry,
) -> Result<(), LeaguePlayerInfoEncodeError> {
    if !matches!(
        player.player_type,
        PLAYER_INFO_TYPE_USER | PLAYER_INFO_TYPE_SCRIPT
    ) {
        return Err(LeaguePlayerInfoEncodeError::InvalidPlayerType {
            player_id: player.id,
            player_type: player.player_type,
        });
    }
    match (
        player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0,
        player.resource.as_ref(),
    ) {
        (true, Some(resource)) => {
            if resource.resource_type > 6 {
                return Err(LeaguePlayerInfoEncodeError::InvalidResourceType(
                    resource.resource_type,
                ));
            }
            if resource.loadable && resource.chunk_size == 0 {
                return Err(LeaguePlayerInfoEncodeError::ZeroResourceChunkSize(
                    resource.id,
                ));
            }
        }
        (true, None) => {
            return Err(LeaguePlayerInfoEncodeError::MissingPlayerResource(
                player.id,
            ));
        }
        (false, Some(_)) => {
            return Err(LeaguePlayerInfoEncodeError::UnexpectedPlayerResource(
                player.id,
            ));
        }
        (false, None) => {}
    }
    Ok(())
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

    /// Applies a successful client-side `Auth` reply to the pending player.
    ///
    /// Native `GetAuthReply` rejects nominally successful replies that omit
    /// the one-use AUID, so callers must not submit such players to the host.
    pub fn apply_player_auth(&self, player: &mut ControlPlayerInfoEntry) -> bool {
        if !self.is_success() || self.auid.is_empty() {
            return false;
        }
        player.auth_id.clone_from(&self.auid);
        true
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

impl LeagueJoinResponse {
    /// Applies one host-side league authentication result to its pending player.
    ///
    /// League values are selected by the exact synchronized league name. The
    /// authentication ID is consumed only when the server accepted the player.
    pub fn apply_auth_check(
        &self,
        league: &LegacyCString,
        player: &mut clonk_engine::ControlPlayerInfoEntry,
    ) -> bool {
        let selected = self
            .leagues
            .iter()
            .position(|candidate| candidate == league);
        player.league_account = validate_name_allow_empty(self.head.account.clone());
        player.league_score = selected.map_or(0, |index| self.scores[index]);
        player.league_rank = selected.map_or(0, |index| self.ranks[index]);
        player.league_rank_symbol = selected.map_or(0, |index| self.rank_symbols[index]);
        player.clan_tag = validate_name_allow_empty(self.clan_tag.clone());
        player.league_progress_data_is_null = selected.is_none();
        player.league_progress_data = selected
            .map(|index| self.progress_data[index].clone())
            .unwrap_or_default();
        let accepted = self.head.is_success();
        if accepted {
            player.auth_id = LegacyCString::default();
        }
        accepted
    }
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

/// Validated reply to `C4LA_Start`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueStartResponse {
    pub head: LeagueAuthResponse,
    pub league: LegacyCString,
    pub stream_to: LegacyCString,
    /// `None` means the `Seed` name was absent and the caller must retain its
    /// existing random seed. `Some(0)` is a real override.
    pub seed: Option<i32>,
    pub max_players: i32,
}

/// Parsed reply to `C4LA_Update`. Native code accepts any response Status once
/// the HTTP exchange and INI parse succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueUpdateResponse {
    pub head: LeagueAuthResponse,
    pub league: LegacyCString,
    pub player_infos: ClientPlayerInfosSnapshot,
}

impl Default for LeagueUpdateResponse {
    fn default() -> Self {
        Self {
            head: LeagueAuthResponse::default(),
            league: LegacyCString::default(),
            player_infos: ClientPlayerInfosSnapshot {
                client_id: -1,
                flags: 0,
                players: Vec::new(),
            },
        }
    }
}

/// Validated reply to `C4LA_End`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueEndResponse {
    pub head: LeagueAuthResponse,
    pub players: Vec<LeagueRoundResultsPlayer>,
}

/// Structural or semantic failure while reading a host registration reply.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeagueResponseDecodeError {
    #[error("league response field `{field}` is not a valid {expected}")]
    InvalidField {
        field: &'static str,
        expected: &'static str,
    },
    #[error("league response contains {0} Player sections, above the C++ limit")]
    PlayerCountOutOfRange(usize),
    #[error("league response contains a loadable resource with zero chunk size")]
    ZeroResourceChunkSize,
    #[error("league Start response did not report Success")]
    StartRejected(Box<LeagueStartResponse>),
    #[error("league Start response reported Success without a CSID")]
    MissingStartCsid(Box<LeagueStartResponse>),
    #[error("league End response did not report Success")]
    EndRejected(Box<LeagueEndResponse>),
}

/// Structural failure while reading C++'s named `C4PlayerInfoList` INI form.
///
/// SavePlayerInfos.txt uses the same nested player parser as league updates,
/// but has a required `[PlayerInfoList]` root and independently bounded Client
/// and Player section counts (`src/C4PlayerInfo.cpp:601-633,1731-1759`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlayerInfoListIniError {
    #[error("player-info INI has no valid [PlayerInfoList] root section")]
    MissingRoot,
    #[error("player-info INI contains {0} Client sections, above the C++ limit")]
    ClientCountOutOfRange(usize),
    #[error("player-info INI contains {0} Player sections, above the C++ limit")]
    PlayerCountOutOfRange(usize),
    #[error("player-info INI field `{field}` is not a valid {expected}")]
    InvalidField {
        field: &'static str,
        expected: &'static str,
    },
    #[error("player-info INI contains a loadable resource with zero chunk size")]
    ZeroResourceChunkSize,
}

/// Decodes a named `C4PlayerInfoList`, as stored in SavePlayerInfos.txt.
///
/// The parser preserves native string bytes, C++ escaped strings, client and
/// player section order, and the raw `LastPlayerID` allocation high-water
/// mark. Missing/malformed scalar values retain the same naming defaults as
/// the shared league-response player parser.
pub fn decode_player_info_list_ini(
    input: &[u8],
) -> Result<PlayerInfoListSnapshot, PlayerInfoListIniError> {
    let tree = LeagueIniTree::parse(input);
    let root = tree
        .first_root_section(b"PlayerInfoList")
        .ok_or(PlayerInfoListIniError::MissingRoot)?;
    let client_nodes = tree.sections(Some(root), b"Client");
    if client_nodes.len() > MAX_PLAYER_INFO_COUNT {
        return Err(PlayerInfoListIniError::ClientCountOutOfRange(
            client_nodes.len(),
        ));
    }
    let last_player_id = tree
        .first_value(Some(root), b"LastPlayerID")
        .map(|raw| parse_i32_response(raw, "PlayerInfoList.LastPlayerID").unwrap_or(0))
        .unwrap_or(0);
    let clients = client_nodes
        .into_iter()
        .map(|node| parse_client_player_infos(&tree, Some(node)).map_err(map_player_info_ini_error))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PlayerInfoListSnapshot {
        last_player_id,
        clients,
    })
}

fn map_player_info_ini_error(error: LeagueResponseDecodeError) -> PlayerInfoListIniError {
    match error {
        LeagueResponseDecodeError::InvalidField { field, expected } => {
            PlayerInfoListIniError::InvalidField { field, expected }
        }
        LeagueResponseDecodeError::PlayerCountOutOfRange(count) => {
            PlayerInfoListIniError::PlayerCountOutOfRange(count)
        }
        LeagueResponseDecodeError::ZeroResourceChunkSize => {
            PlayerInfoListIniError::ZeroResourceChunkSize
        }
        LeagueResponseDecodeError::StartRejected(_)
        | LeagueResponseDecodeError::MissingStartCsid(_)
        | LeagueResponseDecodeError::EndRejected(_) => {
            unreachable!("player-info parsing cannot produce a response-status error")
        }
    }
}

/// Decodes and validates the Start response, preserving C++ Seed-presence
/// semantics and escaped-string handling for League/StreamTo.
pub fn decode_league_start_response(
    input: &[u8],
) -> Result<LeagueStartResponse, LeagueResponseDecodeError> {
    let tree = LeagueIniTree::parse(input);
    let response_node = tree.first_root_section(b"Response");
    let seed = tree
        .first_value(response_node, b"Seed")
        .map(|raw| parse_i32_response(raw, "Seed").unwrap_or(0));
    let response = LeagueStartResponse {
        head: tree.common_response(response_node),
        league: tree.escaped_value(response_node, b"League"),
        stream_to: tree.escaped_value(response_node, b"StreamTo"),
        seed,
        max_players: tree
            .first_value(response_node, b"MaxPlayers")
            .map(|raw| parse_i32_response(raw, "MaxPlayers").unwrap_or(0))
            .unwrap_or(0),
    };
    if !response.head.is_success() {
        return Err(LeagueResponseDecodeError::StartRejected(Box::new(response)));
    }
    if response.head.csid.is_empty() {
        return Err(LeagueResponseDecodeError::MissingStartCsid(Box::new(
            response,
        )));
    }
    Ok(response)
}

/// Decodes Update reply data without imposing a Status check, matching
/// `C4LeagueClient::GetUpdateReply`.
pub fn decode_league_update_response(
    input: &[u8],
) -> Result<LeagueUpdateResponse, LeagueResponseDecodeError> {
    let tree = LeagueIniTree::parse(input);
    let response_node = tree.first_root_section(b"Response");
    let player_infos_node = tree.first_section(response_node, b"PlayerInfos");
    Ok(LeagueUpdateResponse {
        head: tree.common_response(response_node),
        league: tree.escaped_value(response_node, b"League"),
        player_infos: parse_client_player_infos(&tree, player_infos_node)?,
    })
}

/// Decodes End reply round results and validates only the common Status. The
/// native caller deliberately ignores failures from the nested PlayerInfos
/// parse. Rust cannot safely expose the native parser's possibly partial list,
/// so an explicitly corrupt nested structure becomes empty without making an
/// otherwise-successful End fail.
pub fn decode_league_end_response(
    input: &[u8],
) -> Result<LeagueEndResponse, LeagueResponseDecodeError> {
    let tree = LeagueIniTree::parse(input);
    let response_node = tree.first_root_section(b"Response");
    let player_infos_node = tree.first_section(response_node, b"PlayerInfos");
    let players = parse_round_results_players(&tree, player_infos_node).unwrap_or_default();
    let response = LeagueEndResponse {
        head: tree.common_response(response_node),
        players,
    };
    if !response.head.is_success() {
        return Err(LeagueResponseDecodeError::EndRejected(Box::new(response)));
    }
    Ok(response)
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

pub(crate) fn parse_escaped_value(raw: &[u8]) -> (Vec<u8>, &[u8], bool) {
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
            b'x' => {
                let Some((&first, _)) = remaining.split_first() else {
                    output.push(b'x');
                    continue;
                };
                if !first.is_ascii_hexdigit() {
                    output.push(b'x');
                    continue;
                }
                let mut code = 0_i32;
                while let Some((&next, rest)) = remaining.split_first() {
                    if !next.is_ascii_hexdigit() {
                        break;
                    }
                    // Preserve the native reader's lowercase-only alpha
                    // conversion, including its odd uppercase behavior.
                    let digit = if next.is_ascii_digit() {
                        i32::from(next - b'0')
                    } else {
                        i32::from(next) - i32::from(b'a') + 10
                    };
                    code = code.wrapping_mul(16).wrapping_add(digit);
                    remaining = rest;
                }
                output.push(code as u8);
            }
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

#[derive(Debug)]
struct LeagueIniNode<'a> {
    parent: Option<usize>,
    name: &'a [u8],
    value: Option<&'a [u8]>,
    indent: usize,
}

/// Byte-preserving projection of `StdCompilerINIRead::CreateNameTree`.
#[derive(Debug)]
struct LeagueIniTree<'a> {
    nodes: Vec<LeagueIniNode<'a>>,
}

impl<'a> LeagueIniTree<'a> {
    fn parse(input: &'a [u8]) -> Self {
        let input = input.split(|byte| *byte == 0).next().unwrap_or_default();
        let mut nodes = Vec::<LeagueIniNode<'a>>::new();
        let mut section_stack = Vec::<usize>::new();

        for line in input.split(|byte| *byte == b'\r' || *byte == b'\n') {
            let indent = line
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            let content = &line[indent..];
            let (section, name_start) = if content.first() == Some(&b'[')
                && content.get(1).is_some_and(u8::is_ascii_alphabetic)
            {
                (true, 1)
            } else if content.first().is_some_and(u8::is_ascii_alphabetic) {
                (false, 0)
            } else {
                continue;
            };
            let mut name_end = name_start;
            while content
                .get(name_end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
            {
                name_end += 1;
            }
            let mut delimiter = name_end;
            while content
                .get(delimiter)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                delimiter += 1;
            }
            let expected = if section { b']' } else { b'=' };
            if content.get(delimiter) != Some(&expected) {
                continue;
            }

            let effective_indent = indent + usize::from(!section);
            while section_stack
                .last()
                .is_some_and(|index| nodes[*index].indent >= effective_indent)
            {
                section_stack.pop();
            }
            let parent = section_stack.last().copied();
            let index = nodes.len();
            nodes.push(LeagueIniNode {
                parent,
                name: &content[name_start..name_end],
                value: (!section).then_some(&content[delimiter + 1..]),
                indent: effective_indent,
            });
            if section {
                section_stack.push(index);
            }
        }
        Self { nodes }
    }

    fn first_root_section(&self, name: &[u8]) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.parent.is_none() && node.value.is_none() && node.name == name)
    }

    fn first_section(&self, parent: Option<usize>, name: &[u8]) -> Option<usize> {
        let parent = parent?;
        self.nodes.iter().position(|node| {
            node.parent == Some(parent) && node.value.is_none() && node.name == name
        })
    }

    fn sections(&self, parent: Option<usize>, name: &[u8]) -> Vec<usize> {
        let Some(parent) = parent else {
            return Vec::new();
        };
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                (node.parent == Some(parent) && node.value.is_none() && node.name == name)
                    .then_some(index)
            })
            .collect()
    }

    fn first_value(&self, parent: Option<usize>, name: &[u8]) -> Option<&'a [u8]> {
        let parent = parent?;
        self.nodes.iter().find_map(|node| {
            (node.parent == Some(parent) && node.name == name)
                .then_some(node.value)
                .flatten()
        })
    }

    fn escaped_value(&self, parent: Option<usize>, name: &[u8]) -> LegacyCString {
        self.first_value(parent, name)
            .map(decode_cpp_escaped_string)
            .unwrap_or_default()
    }

    fn validated_escaped_value(
        &self,
        parent: Option<usize>,
        name: &[u8],
        validate: fn(LegacyCString) -> LegacyCString,
    ) -> LegacyCString {
        self.first_value(parent, name)
            .map(|raw| validate(decode_cpp_escaped_string(raw)))
            .unwrap_or_default()
    }

    fn identifier_value(&self, parent: Option<usize>, name: &[u8]) -> LegacyCString {
        self.first_value(parent, name)
            .map(|raw| legacy_string(identifier_token(raw)))
            .unwrap_or_default()
    }

    fn raw_value(&self, parent: Option<usize>, name: &[u8]) -> LegacyCString {
        self.first_value(parent, name)
            .map(|raw| legacy_string(trim_horizontal_start(raw)))
            .unwrap_or_default()
    }

    fn validated_raw_value(
        &self,
        parent: Option<usize>,
        name: &[u8],
        validate: fn(LegacyCString) -> LegacyCString,
    ) -> LegacyCString {
        self.first_value(parent, name)
            .map(|raw| validate(legacy_string(trim_horizontal_start(raw))))
            .unwrap_or_default()
    }

    fn common_response(&self, response: Option<usize>) -> LeagueAuthResponse {
        LeagueAuthResponse {
            status: self.identifier_value(response, b"Status"),
            csid: self.identifier_value(response, b"CSID"),
            message: self.raw_value(response, b"Message"),
            account: self.raw_value(response, b"Account"),
            auid: self.raw_value(response, b"AUID"),
            fbid: self.raw_value(response, b"FBID"),
        }
    }
}

fn decode_cpp_escaped_string(raw: &[u8]) -> LegacyCString {
    // StdCompiler's std::string compatibility fallback checks the byte
    // immediately after '=' before the reader skips whitespace.
    if raw.first() != Some(&b'"') {
        return legacy_string(trim_horizontal_start(raw));
    }
    let (mut decoded, _, _) = parse_escaped_value(raw);
    if let Some(nul) = decoded.iter().position(|byte| *byte == 0) {
        decoded.truncate(nul);
    }
    LegacyCString::from_bytes(decoded).expect("decoded string was truncated at its first NUL")
}

fn invalid_field(field: &'static str, expected: &'static str) -> LeagueResponseDecodeError {
    LeagueResponseDecodeError::InvalidField { field, expected }
}

fn numeric_token(raw: &[u8]) -> Option<(&[u8], bool, u32)> {
    let raw = trim_horizontal_start(raw);
    let mut offset = 0;
    let negative = match raw.first() {
        Some(b'-') => {
            offset = 1;
            true
        }
        Some(b'+') => {
            offset = 1;
            false
        }
        _ => false,
    };
    let (radix, prefix) = if raw
        .get(offset..offset + 2)
        .is_some_and(|prefix| prefix[0] == b'0' && matches!(prefix[1], b'x' | b'X'))
    {
        (16, 2)
    } else {
        (10, 0)
    };
    offset += prefix;
    let digit_start = offset;
    while raw.get(offset).is_some_and(|byte| match radix {
        16 => byte.is_ascii_hexdigit(),
        _ => byte.is_ascii_digit(),
    }) {
        offset += 1;
    }
    (offset > digit_start).then_some((&raw[digit_start..offset], negative, radix))
}

fn parse_i32_response(raw: &[u8], field: &'static str) -> Result<i32, LeagueResponseDecodeError> {
    let (digits, negative, radix) =
        numeric_token(raw).ok_or_else(|| invalid_field(field, "signed integer"))?;
    let digits = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| i64::from_str_radix(digits, radix).ok())
        .ok_or_else(|| invalid_field(field, "signed integer"))?;
    let value = if negative { -digits } else { digits };
    i32::try_from(value).map_err(|_| invalid_field(field, "signed 32-bit integer"))
}

fn parse_u32_response(raw: &[u8], field: &'static str) -> Result<u32, LeagueResponseDecodeError> {
    let (digits, negative, radix) =
        numeric_token(raw).ok_or_else(|| invalid_field(field, "unsigned integer"))?;
    if negative {
        return Err(invalid_field(field, "unsigned integer"));
    }
    let digits = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| u64::from_str_radix(digits, radix).ok())
        .ok_or_else(|| invalid_field(field, "unsigned integer"))?;
    u32::try_from(digits).map_err(|_| invalid_field(field, "unsigned 32-bit integer"))
}

fn parse_bool_response(raw: &[u8], field: &'static str) -> Result<bool, LeagueResponseDecodeError> {
    match trim_horizontal_end(trim_horizontal_start(raw)) {
        b"1" | b"true" => Ok(true),
        b"0" | b"false" => Ok(false),
        _ => Err(invalid_field(field, "boolean")),
    }
}

fn identifier_token(raw: &[u8]) -> &[u8] {
    let raw = trim_horizontal_start(raw);
    let end = raw
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
        .unwrap_or(raw.len());
    &raw[..end]
}

fn parse_bitfield(raw: &[u8], names: &[(&[u8], u32)]) -> u32 {
    let mut result = 0;
    for token in raw.split(|byte| *byte == b'|') {
        let token = trim_horizontal_end(trim_horizontal_start(token));
        if let Some((_, value)) = names.iter().find(|(name, _)| *name == token) {
            result |= value;
        } else if let Some((digits, negative, radix)) = numeric_token(token) {
            if let Some(magnitude) = std::str::from_utf8(digits)
                .ok()
                .and_then(|digits| u32::from_str_radix(digits, radix).ok())
            {
                let value = if negative {
                    0_u32.wrapping_sub(magnitude)
                } else {
                    magnitude
                };
                result |= value;
            }
        }
    }
    result
}

fn parse_client_player_infos(
    tree: &LeagueIniTree<'_>,
    node: Option<usize>,
) -> Result<ClientPlayerInfosSnapshot, LeagueResponseDecodeError> {
    let players = tree.sections(node, b"Player");
    if players.len() > MAX_PLAYER_INFO_COUNT {
        return Err(LeagueResponseDecodeError::PlayerCountOutOfRange(
            players.len(),
        ));
    }
    let client_id = tree
        .first_value(node, b"ID")
        .map(|raw| parse_i32_response(raw, "PlayerInfos.ID").unwrap_or(-1))
        .unwrap_or(-1);
    let flags = tree
        .first_value(node, b"Flags")
        .map(|raw| {
            parse_bitfield(
                raw,
                &[
                    (b"AddPlayers", CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS),
                    (b"Updated", CLIENT_PLAYER_INFO_FLAG_UPDATED),
                    (b"Initial", CLIENT_PLAYER_INFO_FLAG_INITIAL),
                ],
            )
        })
        .unwrap_or(0);
    let players = players
        .into_iter()
        .map(|player| parse_player_info(tree, player))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClientPlayerInfosSnapshot {
        client_id,
        flags,
        players,
    })
}

fn parse_player_info(
    tree: &LeagueIniTree<'_>,
    node: usize,
) -> Result<ControlPlayerInfoEntry, LeagueResponseDecodeError> {
    let node = Some(node);
    let mut flags = tree
        .first_value(node, b"Flags")
        .map(|raw| {
            parse_bitfield(
                raw,
                &[
                    (b"Joined", u32::from(PLAYER_INFO_FLAG_JOINED)),
                    (b"Removed", u32::from(PLAYER_INFO_FLAG_REMOVED)),
                    (b"HasResource", u32::from(PLAYER_INFO_FLAG_HAS_RESOURCE)),
                    (b"JoinIssued", u32::from(PLAYER_INFO_FLAG_JOIN_ISSUED)),
                    (
                        b"InScenarioFile",
                        u32::from(PLAYER_INFO_FLAG_IN_SCENARIO_FILE),
                    ),
                    (b"SavegameJoin", u32::from(PLAYER_INFO_FLAG_SAVEGAME_JOIN)),
                    (b"Disconnected", u32::from(PLAYER_INFO_FLAG_DISCONNECTED)),
                    (b"VotedOut", u32::from(PLAYER_INFO_FLAG_VOTED_OUT)),
                    (b"Won", u32::from(PLAYER_INFO_FLAG_WON)),
                    (
                        b"AttributesFixed",
                        u32::from(PLAYER_INFO_FLAG_ATTRIBUTES_FIXED),
                    ),
                    (
                        b"NoScenarioInit",
                        u32::from(PLAYER_INFO_FLAG_NO_SCENARIO_INIT),
                    ),
                    (
                        b"NoEliminationCheck",
                        u32::from(PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK),
                    ),
                    (b"Invisible", u32::from(PLAYER_INFO_FLAG_INVISIBLE)),
                ],
            )
        })
        .unwrap_or(0)
        .min(u32::from(u16::MAX)) as u16;
    let player_type = match tree.first_value(node, b"Type") {
        None => PLAYER_INFO_TYPE_USER,
        Some(raw) => {
            let token = identifier_token(raw);
            if token == b"User" {
                PLAYER_INFO_TYPE_USER
            } else if token == b"Script" {
                PLAYER_INFO_TYPE_SCRIPT
            } else {
                parse_u32_response(raw, "Player.Type")
                    .map(|value| value.min(u32::from(u8::MAX)) as u8)
                    .unwrap_or(PLAYER_INFO_TYPE_USER)
            }
        }
    };
    if player_type != PLAYER_INFO_TYPE_SCRIPT {
        flags &= !PLAYER_INFO_FLAG_INVISIBLE;
    }
    let color = tree
        .first_value(node, b"Color")
        .map(|raw| parse_u32_response(raw, "Player.Color").unwrap_or(0))
        .unwrap_or(0);
    let int = |name: &[u8], field: &'static str, default: i32| {
        tree.first_value(node, name)
            .map(|raw| parse_i32_response(raw, field).unwrap_or(default))
            .unwrap_or(default)
    };
    let uint = |name: &[u8], field: &'static str, default: u32| {
        tree.first_value(node, name)
            .map(|raw| parse_u32_response(raw, field).unwrap_or(default))
            .unwrap_or(default)
    };
    let extra_data = tree
        .first_value(node, b"ExtraData")
        .map(parse_c4_id)
        .unwrap_or(*b"NONE");
    let resource = if flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
        Some(parse_network_resource(
            tree,
            tree.first_section(node, b"ResCore"),
        )?)
    } else {
        None
    };
    Ok(ControlPlayerInfoEntry {
        name: tree.validated_escaped_value(node, b"Name", validate_name_no_empty),
        forced_name: tree.validated_escaped_value(node, b"ForcedName", validate_name_allow_empty),
        filename: tree.escaped_value(node, b"Filename"),
        flags,
        id: int(b"ID", "Player.ID", 0),
        player_type,
        color,
        original_color: uint(b"OriginalColor", "Player.OriginalColor", color),
        savegame_player: int(b"SavgamePlayer", "Player.SavgamePlayer", 0),
        team: int(b"Team", "Player.Team", 0),
        auth_id: tree.escaped_value(node, b"AUID"),
        game_number: if flags & PLAYER_INFO_FLAG_JOINED != 0 {
            int(b"GameNumber", "Player.GameNumber", -1)
        } else {
            -1
        },
        game_join_frame: if flags & PLAYER_INFO_FLAG_JOINED != 0 {
            int(b"GameJoinFrame", "Player.GameJoinFrame", -1)
        } else {
            -1
        },
        game_part_frame: if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
            int(b"GamePartFrame", "Player.GamePartFrame", -1)
        } else {
            -1
        },
        extra_data,
        league_account: tree.validated_escaped_value(
            node,
            b"LeagueAccount",
            validate_name_allow_empty,
        ),
        league_score: int(b"LeagueScore", "Player.LeagueScore", 0),
        league_rank: int(b"LeagueRank", "Player.LeagueRank", 0),
        league_rank_symbol: int(b"LeagueRankSymbol", "Player.LeagueRankSymbol", 0),
        league_projected_gain: int(b"ProjectedGain", "Player.ProjectedGain", -1),
        clan_tag: tree.validated_raw_value(node, b"ClanTag", validate_name_allow_empty),
        league_performance: int(b"LeaguePerformance", "Player.LeaguePerformance", 0),
        league_progress_data_is_null: false,
        league_progress_data: tree.escaped_value(node, b"LeagueProgressData"),
        resource,
    })
}

fn parse_c4_id(raw: &[u8]) -> [u8; 4] {
    let raw = trim_horizontal_start(raw);
    let length = raw
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    if length < 4 {
        *b"NONE"
    } else {
        let id = raw[..4].try_into().expect("slice length was checked");
        if id == *b"0000" {
            *b"NONE"
        } else {
            id
        }
    }
}

fn parse_network_resource(
    tree: &LeagueIniTree<'_>,
    node: Option<usize>,
) -> Result<NetworkResourceCore, LeagueResponseDecodeError> {
    let resource_type = match tree.first_value(node, b"Type") {
        None => NETWORK_RESOURCE_TYPE_NULL,
        Some(raw) => match identifier_token(raw) {
            b"Scenario" => 1,
            b"Dynamic" => 2,
            b"Player" => 3,
            b"Definitions" => 4,
            b"System" => 5,
            b"Material" => 6,
            _ => parse_u32_response(raw, "ResCore.Type")
                .map(|value| value.min(u32::from(u8::MAX)) as u8)
                .unwrap_or(NETWORK_RESOURCE_TYPE_NULL),
        },
    };
    let loadable = tree
        .first_value(node, b"Loadable")
        .map(|raw| parse_bool_response(raw, "ResCore.Loadable").unwrap_or(true))
        .unwrap_or(true);
    let int = |name: &[u8], field: &'static str, default: i32| {
        tree.first_value(node, name)
            .map(|raw| parse_i32_response(raw, field).unwrap_or(default))
            .unwrap_or(default)
    };
    let uint = |name: &[u8], field: &'static str, default: u32| {
        tree.first_value(node, name)
            .map(|raw| parse_u32_response(raw, field).unwrap_or(default))
            .unwrap_or(default)
    };
    let chunk_size = if loadable {
        uint(
            b"ChunkSize",
            "ResCore.ChunkSize",
            NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE,
        )
    } else {
        NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE
    };
    if loadable && chunk_size == 0 {
        return Err(LeagueResponseDecodeError::ZeroResourceChunkSize);
    }
    let file_sha = tree
        .first_value(node, b"FileSHA")
        .map(parse_sha1)
        .transpose()?;
    Ok(NetworkResourceCore {
        resource_type,
        id: int(b"ID", "ResCore.ID", -1),
        derived_id: int(b"DerID", "ResCore.DerID", -1),
        loadable,
        file_size: if loadable {
            uint(b"FileSize", "ResCore.FileSize", 0)
        } else {
            u32::MAX
        },
        file_crc: if loadable {
            uint(b"FileCRC", "ResCore.FileCRC", 0)
        } else {
            u32::MAX
        },
        chunk_size,
        contents_crc: uint(b"ContentsCRC", "ResCore.ContentsCRC", 0),
        file_sha,
        filename: normalize_network_filename(tree.escaped_value(node, b"Filename")),
        author: normalize_network_filename(tree.escaped_value(node, b"Author")),
    })
}

fn parse_sha1(raw: &[u8]) -> Result<[u8; 20], LeagueResponseDecodeError> {
    let raw = trim_horizontal_start(raw);
    if raw.len() < 40 {
        return Err(invalid_field("ResCore.FileSHA", "40 hexadecimal digits"));
    }
    let mut digest = [0; 20];
    for (index, byte) in digest.iter_mut().enumerate() {
        let pair = &raw[index * 2..index * 2 + 2];
        *byte = std::str::from_utf8(pair)
            .ok()
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .ok_or_else(|| invalid_field("ResCore.FileSHA", "40 hexadecimal digits"))?;
    }
    Ok(digest)
}

#[cfg(windows)]
fn normalize_network_filename(value: LegacyCString) -> LegacyCString {
    value
}

#[cfg(not(windows))]
fn normalize_network_filename(value: LegacyCString) -> LegacyCString {
    LegacyCString::from_bytes(
        value
            .as_bytes()
            .iter()
            .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
            .collect(),
    )
    .expect("normalization cannot introduce NUL")
}

fn parse_round_results_players(
    tree: &LeagueIniTree<'_>,
    node: Option<usize>,
) -> Result<Vec<LeagueRoundResultsPlayer>, LeagueResponseDecodeError> {
    let players = tree.sections(node, b"Player");
    if players.len() > MAX_PLAYER_INFO_COUNT {
        return Err(LeagueResponseDecodeError::PlayerCountOutOfRange(
            players.len(),
        ));
    }
    players
        .into_iter()
        .map(|node| parse_round_results_player(tree, node))
        .collect()
}

fn parse_round_results_player(
    tree: &LeagueIniTree<'_>,
    node: usize,
) -> Result<LeagueRoundResultsPlayer, LeagueResponseDecodeError> {
    let node = Some(node);
    let int = |name: &[u8], field: &'static str, default: i32| {
        tree.first_value(node, name)
            .map(|raw| parse_i32_response(raw, field).unwrap_or(default))
            .unwrap_or(default)
    };
    let total_playing_time = tree
        .first_value(node, b"TotalPlayingTime")
        .map(|raw| parse_u32_response(raw, "Player.TotalPlayingTime").unwrap_or(0))
        .unwrap_or(0);
    let status = match tree.first_value(node, b"Status") {
        None => LeagueRoundPlayerStatus::Unknown,
        Some(raw) => {
            let token = identifier_token(raw);
            if token == b"Lost" {
                LeagueRoundPlayerStatus::Lost
            } else if token == b"Won" {
                LeagueRoundPlayerStatus::Won
            } else if let Ok(value) = parse_u32_response(raw, "Player.Status") {
                LeagueRoundPlayerStatus::from(value.min(u32::from(u8::MAX)) as u8)
            } else {
                LeagueRoundPlayerStatus::Unknown
            }
        }
    };
    Ok(LeagueRoundResultsPlayer {
        player_info_id: int(b"ID", "Player.ID", 0),
        total_playing_time,
        settlement_score_old: int(b"SettlementScoreOld", "Player.SettlementScoreOld", -1),
        settlement_score_new: int(b"SettlementScoreNew", "Player.SettlementScoreNew", -1),
        league_score_new: int(b"Score", "Player.Score", -1),
        league_score_gain: int(b"GameScore", "Player.GameScore", -1),
        league_rank_new: int(b"Rank", "Player.Rank", 0),
        league_rank_symbol_new: int(b"RankSymbol", "Player.RankSymbol", 0),
        league_progress_data: tree.escaped_value(node, b"LeagueProgressData"),
        status,
    })
}

/// Tracks league feedback identifiers (FBIDs) for authenticated accounts.
///
/// The classic runtime keeps a linked list that is synchronised with the list of
/// players reported to the league backend. Consumers look up FBIDs by account
/// name when constructing disconnect reports or when restoring cached
/// authentication state.  This registry mirrors the semantics of
/// `C4LeagueFBIDList`: inserting a new FBID replaces any previous mapping for
/// the account and removal is a no-op if the entry does not exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueFbidRegistry {
    entries: HashMap<LegacyCString, LegacyCString>,
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
    pub fn insert(&mut self, account: LegacyCString, fbid: LegacyCString) {
        self.entries.insert(account, fbid);
    }

    /// Removes the FBID associated with `account`.
    ///
    /// Returns `true` if an entry was present.
    pub fn remove(&mut self, account: &LegacyCString) -> bool {
        self.entries.remove(account).is_some()
    }

    /// Looks up the FBID registered for `account`.
    pub fn get(&self, account: &LegacyCString) -> Option<&LegacyCString> {
        self.entries.get(account)
    }

    pub fn extend_from(&mut self, other: &Self) {
        self.entries.extend(
            other
                .entries
                .iter()
                .map(|(account, fbid)| (account.clone(), fbid.clone())),
        );
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
        decode_league_auth_response, decode_league_end_response, decode_league_join_response,
        decode_league_report_disconnect_response, decode_league_start_response,
        decode_league_update_response, decode_player_info_list_ini, encode_league_auth_request,
        encode_league_auth_request_head, encode_league_join_request_head,
        encode_league_player_info_section, encode_league_report_disconnect_request,
        solve_league_checksum, LeagueAuthRequestHead, LeagueDisconnectReason, LeagueFbidRegistry,
        LeagueHostSession, LeagueHttpPostTransport, LeagueHttpTransportConfig,
        LeagueHttpTransportError, LeagueIniTree, LeagueJoinRequestHead, LeagueResponseDecodeError,
        PlayerInfoListIniError,
    };
    use crate::{ClientPlayerInfosSnapshot, LeagueRoundPlayerStatus};
    use clonk_engine::{
        ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
        CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS, CLIENT_PLAYER_INFO_FLAG_INITIAL,
        CLIENT_PLAYER_INFO_FLAG_UPDATED, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
        PLAYER_INFO_FLAG_IN_SCENARIO_FILE, PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_REMOVED,
        PLAYER_INFO_TYPE_SCRIPT,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

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
    fn report_disconnect_request_matches_cpp_bytes_and_complete_checksum() {
        // ReportDisconnect inserts Request and PlayerInfos as top-level
        // siblings. DisconnectData emits only joined/non-removed players and
        // looks up each optional FBID by its exact league-account bytes
        // (src/C4League.cpp:74-89,247-286,483-497).
        let alice_account = legacy(b"A\x80");
        let bob_account = legacy(b"Bob");
        let mut fbids = LeagueFbidRegistry::new();
        fbids.insert(alice_account.clone(), legacy(b"feedback-\x81"));
        let players = ClientPlayerInfosSnapshot {
            client_id: 7,
            flags: 0,
            players: vec![
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_JOINED,
                    id: 17,
                    league_account: alice_account,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_JOINED,
                    id: 18,
                    league_account: bob_account,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED,
                    id: 19,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 20,
                    ..Default::default()
                },
            ],
        };

        assert_eq!(
            encode_league_report_disconnect_request(
                &legacy(b"session-\x82"),
                LeagueDisconnectReason::ConnectionFailed,
                &players,
                &fbids,
                0x1234_5678,
            )
            .expect("disconnect checksum has a C++ candidate"),
            b"[Request]\r\n\
Action=ReportDisconnect\r\n\
CSID=session-\x82\r\n\
Checksum=8HSoj\r\n\
Reason=ConnectionFailed\r\n\
\r\n\
[PlayerInfos]\r\n\
\r\n\
\x20\x20[Player]\r\n\
\x20\x20ID=17\r\n\
\x20\x20FBID=feedback-\x81\r\n\
\r\n\
\x20\x20[Player]\r\n\
\x20\x20ID=18\r\n"
        );
    }

    #[test]
    fn report_disconnect_omits_empty_player_infos_and_unknown_reason() {
        // StdCompilerINIWrite cannot represent an empty named section, so an
        // all-filtered PlayerInfos sibling is omitted together with the
        // default Unknown reason (src/StdCompiler.cpp:248-280).
        let request = encode_league_report_disconnect_request(
            &LegacyCString::default(),
            LeagueDisconnectReason::Unknown,
            &ClientPlayerInfosSnapshot {
                client_id: 4,
                flags: 0,
                players: vec![ControlPlayerInfoEntry {
                    id: 9,
                    ..Default::default()
                }],
            },
            &LeagueFbidRegistry::new(),
            0,
        )
        .expect("disconnect checksum has a C++ candidate");

        assert_eq!(
            request,
            b"[Request]\r\nAction=ReportDisconnect\r\nChecksum=B4FAA\r\n"
        );
    }

    #[test]
    fn report_disconnect_desync_empty_fbid_and_reply_follow_cpp_semantics() {
        let account = legacy(b"A \x80\\");
        let mut fbids = LeagueFbidRegistry::new();
        fbids.insert(account.clone(), LegacyCString::default());
        let request = encode_league_report_disconnect_request(
            &legacy(b"session"),
            LeagueDisconnectReason::Desync,
            &ClientPlayerInfosSnapshot {
                client_id: 4,
                flags: 0,
                players: vec![ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_JOINED,
                    id: -7,
                    league_account: account,
                    ..Default::default()
                }],
            },
            &fbids,
            0,
        )
        .expect("disconnect checksum has a C++ candidate");
        assert!(request
            .windows(b"Reason=Desync\r\n".len())
            .any(|window| window == b"Reason=Desync\r\n"));
        assert!(request
            .windows(b"  ID=-7\r\n  FBID=\r\n".len())
            .any(|window| window == b"  ID=-7\r\n  FBID=\r\n"));

        let success = decode_league_report_disconnect_response(
            b"[Response]\r\nStatus=sUcCeSs!ignored\r\nMessage= Recorded \x80  \r\n",
        );
        assert!(success.is_success());
        assert_eq!(success.message.as_bytes(), b"Recorded \x80  ");
        let failure = decode_league_report_disconnect_response(
            b"[Response]\r\nStatus=SuccessExtra\r\nMessage=Rejected\r\n",
        );
        assert!(!failure.is_success());
        assert_eq!(failure.message.as_bytes(), b"Rejected");
    }

    #[test]
    fn league_player_info_section_matches_cpp_ini_vector() {
        // AuthCheck inserts mkDecompileAdapt(C4PlayerInfo) as the sibling
        // [PlrInfo] section after [Request] (pristine 9ffa0a5d
        // src/C4League.cpp:451-466). C4PlayerInfo and its ResCore compile in
        // this exact field/default order (src/C4PlayerInfo.cpp:177-268;
        // src/C4Network2Res.cpp:114-143; src/StdCompiler.cpp:248-485).
        let player = ControlPlayerInfoEntry {
            name: legacy(b"A\x80\""),
            filename: legacy(b"Players/Alice.c4p"),
            flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
            id: 17,
            color: 0x12_34_56,
            original_color: 0x12_34_56,
            team: 2,
            auth_id: legacy(b"auth-\x80"),
            resource: Some(NetworkResourceCore {
                resource_type: 3,
                id: 65_537,
                derived_id: -1,
                loadable: true,
                file_size: 3,
                file_crc: 0x1122_3344,
                chunk_size: 100 * 1024,
                contents_crc: 0x5566_7788,
                file_sha: Some(std::array::from_fn(|index| index as u8)),
                filename: legacy(b"Players/Alice.c4p"),
                author: legacy(b"Maker\x80"),
            }),
            ..Default::default()
        };

        assert_eq!(
            encode_league_player_info_section(&player).expect("valid C++ player info"),
            b"[PlrInfo]\r\n\
Name=\"A\\200\\\"\"\r\n\
Filename=\"Players/Alice.c4p\"\r\n\
Flags=HasResource\r\n\
ID=17\r\n\
Color=1193046\r\n\
Team=2\r\n\
AUID=\"auth-\\200\"\r\n\
\r\n\
\x20\x20[ResCore]\r\n\
\x20\x20Type=Player\r\n\
\x20\x20ID=65537\r\n\
\x20\x20FileSize=3\r\n\
\x20\x20FileCRC=287454020\r\n\
\x20\x20ContentsCRC=1432778632\r\n\
\x20\x20FileSHA=000102030405060708090a0b0c0d0e0f10111213\r\n\
\x20\x20Filename=\"Players\\\\Alice.c4p\"\r\n\
\x20\x20Author=\"Maker\\200\"\r\n"
        );
    }

    #[test]
    fn league_player_info_clan_tag_round_trips_through_rct_all() {
        let player = ControlPlayerInfoEntry {
            color: 0,
            original_color: 0,
            clan_tag: legacy(b"Cl\xe4n"),
            ..Default::default()
        };

        let encoded = encode_league_player_info_section(&player).expect("valid C++ player info");

        assert_eq!(encoded, b"[PlrInfo]\r\nClanTag=Cl\xe4n\r\n");
        let tree = LeagueIniTree::parse(&encoded);
        let player_info = tree.first_root_section(b"PlrInfo");
        assert_eq!(
            tree.raw_value(player_info, b"ClanTag").as_bytes(),
            b"Cl\xe4n"
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
    fn client_auth_requires_success_with_a_nonempty_auid() {
        // GetAuthReply accepts a local player only after both Status=Success
        // and a nonempty AUID, then JoinLocalPlayer carries that exact token
        // to the host (src/C4League.cpp:423-448;
        // src/C4Network2.cpp:2680-2688).
        let mut player = clonk_engine::ControlPlayerInfoEntry::default();
        let success =
            decode_league_auth_response(b"[Response]\r\nStatus=Success\r\nAUID=one-use-token\r\n");
        assert!(success.apply_player_auth(&mut player));
        assert_eq!(player.auth_id.as_bytes(), b"one-use-token");

        let empty = decode_league_auth_response(b"[Response]\r\nStatus=Success\r\nAUID=\r\n");
        assert!(!empty.apply_player_auth(&mut player));
        assert_eq!(player.auth_id.as_bytes(), b"one-use-token");

        let rejected =
            decode_league_auth_response(b"[Response]\r\nStatus=Failure\r\nAUID=unused-token\r\n");
        assert!(!rejected.apply_player_auth(&mut player));
        assert_eq!(player.auth_id.as_bytes(), b"one-use-token");
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
    fn start_response_preserves_seed_presence_and_saves_only_valid_csid() {
        // Start's League and StreamTo are ordinary escaped StdStrBuf values,
        // while Seed uses NameCount so absent and explicit zero differ
        // (src/C4League.cpp:116-129,307-334).
        let mut session = LeagueHostSession::new();
        let response = session
            .accept_start_response(
                b"[Response]\r\n\
Status=Success\r\n\
CSID=session-7.trailing\r\n\
Message= Registered  \r\n\
League=\"Cup, \\200\"\r\n\
StreamTo=\"stream\\x2fround\"\r\n\
MaxPlayers=6\r\n",
            )
            .expect("successful Start response");

        assert_eq!(response.head.csid.as_bytes(), b"session-7");
        assert_eq!(response.head.message.as_bytes(), b"Registered  ");
        assert_eq!(response.league.as_bytes(), b"Cup, \x80");
        assert_eq!(response.stream_to.as_bytes(), b"stream/round");
        assert_eq!(response.seed, None);
        assert_eq!(response.max_players, 6);
        assert_eq!(session.csid(), Some(&response.head.csid));

        let response = decode_league_start_response(
            b"[Response]\r\nStatus=Success\r\nCSID=zero\r\nSeed=0\r\n",
        )
        .expect("explicit zero seed remains present");
        assert_eq!(response.seed, Some(0));

        let response = decode_league_start_response(
            b"[Response]\r\nStatus=Success\r\nCSID=defaults\r\nSeed=broken\r\nMaxPlayers=broken\r\n",
        )
        .expect("malformed defaulted numbers use their naming defaults");
        assert_eq!((response.seed, response.max_players), (Some(0), 0));

        assert!(matches!(
            decode_league_start_response(
                b"[Response]\r\nStatus=Failure\r\nCSID=unused\r\nMessage=No\r\n"
            ),
            Err(LeagueResponseDecodeError::StartRejected(response))
                if response.head.message.as_bytes() == b"No"
        ));
        assert!(matches!(
            decode_league_start_response(b"[Response]\r\nStatus=Success\r\n"),
            Err(LeagueResponseDecodeError::MissingStartCsid(_))
        ));
    }

    #[test]
    fn player_info_list_ini_decodes_ordered_nested_players_and_native_bytes() {
        // C4PlayerInfoList::Load applies the named PlayerInfoList wrapper and
        // compiles repeated Client/Player sections in source order. Strings
        // use the same byte-preserving escaped reader as league PlayerInfos
        // (src/C4PlayerInfo.cpp:177-268,601-633,1165-1182,1731-1759).
        let snapshot = decode_player_info_list_ini(
            b"[PlayerInfoList]\r\n\
LastPlayerID=41\r\n\
\r\n\
\x20\x20[Client]\r\n\
\x20\x20ID=3\r\n\
\x20\x20Flags=Initial\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20Name=\"Ren\\200\\\"\"\r\n\
\x20\x20\x20\x20ForcedName=Native \x81\r\n\
\x20\x20\x20\x20Filename=\"Players\\\\Ren\\200.c4p\"\r\n\
\x20\x20\x20\x20Flags=Joined|HasResource|InScenarioFile|Invisible\r\n\
\x20\x20\x20\x20ID=7\r\n\
\x20\x20\x20\x20Type=User\r\n\
\x20\x20\x20\x20Color=1193046\r\n\
\x20\x20\x20\x20OriginalColor=6636321\r\n\
\x20\x20\x20\x20SavgamePlayer=5\r\n\
\x20\x20\x20\x20Team=2\r\n\
\x20\x20\x20\x20GameNumber=4\r\n\
\x20\x20\x20\x20GameJoinFrame=23\r\n\
\x20\x20\x20\x20ExtraData=TEST\r\n\
\r\n\
\x20\x20\x20\x20\x20\x20[ResCore]\r\n\
\x20\x20\x20\x20\x20\x20Type=Player\r\n\
\x20\x20\x20\x20\x20\x20ID=17\r\n\
\x20\x20\x20\x20\x20\x20Filename=\"Players\\\\Ren\\200.c4p\"\r\n\
\x20\x20\x20\x20\x20\x20Author=Host \x82\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20Name=Script\r\n\
\x20\x20\x20\x20Flags=Invisible\r\n\
\x20\x20\x20\x20ID=8\r\n\
\x20\x20\x20\x20Type=Script\r\n\
\r\n\
\x20\x20[Client]\r\n\
\x20\x20ID=4\r\n\
\x20\x20Flags=AddPlayers\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20Name=Zo\xeb\r\n\
\x20\x20\x20\x20ID=9\r\n",
        )
        .expect("valid SavePlayerInfos-style INI");

        assert_eq!(snapshot.last_player_id, 41);
        assert_eq!(snapshot.clients.len(), 2);
        assert_eq!(
            snapshot
                .clients
                .iter()
                .map(|client| (client.client_id, client.flags))
                .collect::<Vec<_>>(),
            vec![
                (3, CLIENT_PLAYER_INFO_FLAG_INITIAL),
                (4, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS),
            ]
        );
        assert_eq!(
            snapshot.clients[0]
                .players
                .iter()
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![7, 8]
        );
        assert_eq!(snapshot.clients[1].players[0].id, 9);

        let user = &snapshot.clients[0].players[0];
        assert_eq!(user.name.as_bytes(), b"Ren\x80\"");
        assert_eq!(user.forced_name.as_bytes(), b"Native \x81");
        assert_eq!(user.filename.as_bytes(), b"Players\\Ren\x80.c4p");
        assert_eq!(
            user.flags,
            PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_HAS_RESOURCE
                | PLAYER_INFO_FLAG_IN_SCENARIO_FILE,
            "Invisible is cleared for a user player, but InScenarioFile survives"
        );
        assert_eq!((user.color, user.original_color), (1_193_046, 6_636_321));
        assert_eq!((user.savegame_player, user.team), (5, 2));
        assert_eq!((user.game_number, user.game_join_frame), (4, 23));
        assert_eq!(user.extra_data, *b"TEST");
        let resource = user.resource.as_ref().expect("HasResource parses ResCore");
        assert_eq!((resource.resource_type, resource.id), (3, 17));
        #[cfg(windows)]
        assert_eq!(resource.filename.as_bytes(), b"Players\\Ren\x80.c4p");
        #[cfg(not(windows))]
        assert_eq!(resource.filename.as_bytes(), b"Players/Ren\x80.c4p");
        assert_eq!(resource.author.as_bytes(), b"Host \x82");

        let script = &snapshot.clients[0].players[1];
        assert_eq!(script.player_type, PLAYER_INFO_TYPE_SCRIPT);
        assert_eq!(script.flags, PLAYER_INFO_FLAG_INVISIBLE);
        assert_eq!(snapshot.clients[1].players[0].name.as_bytes(), b"Zo\xeb");
    }

    #[test]
    fn player_info_list_ini_requires_a_well_formed_named_root() {
        for input in [
            b"".as_slice(),
            b"[Wrong]\nLastPlayerID=7\n".as_slice(),
            b"[PlayerInfoList\nLastPlayerID=7\n".as_slice(),
        ] {
            assert_eq!(
                decode_player_info_list_ini(input),
                Err(PlayerInfoListIniError::MissingRoot)
            );
        }
    }

    #[test]
    fn player_info_list_ini_enforces_cpp_client_and_player_limits() {
        // C4MaxClient and C4MaxPlayer are both 5000, independently checked by
        // C4PlayerInfoList and each C4ClientPlayerInfos during named compile
        // (src/C4Player.h:33-34; src/C4PlayerInfo.cpp:618-622,1743-1747).
        let mut too_many_clients = b"[PlayerInfoList]\n".to_vec();
        for _ in 0..=5_000 {
            too_many_clients.extend_from_slice(b"  [Client]\n");
        }
        assert_eq!(
            decode_player_info_list_ini(&too_many_clients),
            Err(PlayerInfoListIniError::ClientCountOutOfRange(5_001))
        );

        let mut too_many_players = b"[PlayerInfoList]\n  [Client]\n".to_vec();
        for _ in 0..=5_000 {
            too_many_players.extend_from_slice(b"    [Player]\n");
        }
        assert_eq!(
            decode_player_info_list_ini(&too_many_players),
            Err(PlayerInfoListIniError::PlayerCountOutOfRange(5_001))
        );
    }

    #[test]
    fn update_response_decodes_cpp_client_player_infos_and_ignores_status() {
        // GetUpdateReply accepts the parsed response regardless of Status and
        // copies one C4ClientPlayerInfos including repeated Player/ResCore
        // sections (src/C4League.cpp:132-142,354-367).
        let response = decode_league_update_response(
            b"[Response]\r\n\
Status=Failure\r\n\
Message=Still parsed  \r\n\
League=\"Cup \\200\"\r\n\
\r\n\
\x20\x20[PlayerInfos]\r\n\
\x20\x20ID=9\r\n\
\x20\x20Flags=Updated|Initial\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20Name=\" {<i>Alice</i>{ \"\r\n\
\x20\x20\x20\x20ForcedName=\" Bob \"\r\n\
\x20\x20\x20\x20Filename=\"Players\\\\Alice.c4p\"\r\n\
\x20\x20\x20\x20Flags=Joined|HasResource|64\r\n\
\x20\x20\x20\x20ID=17\r\n\
\x20\x20\x20\x20Color=1193046\r\n\
\x20\x20\x20\x20Team=2\r\n\
\x20\x20\x20\x20GameNumber=3\r\n\
\x20\x20\x20\x20GameJoinFrame=44\r\n\
\x20\x20\x20\x20LeagueAccount=\" {<i>Alice</i>{ \"\r\n\
\x20\x20\x20\x20ProjectedGain=-7\r\n\
\x20\x20\x20\x20ClanTag= TAG  \r\n\
\x20\x20\x20\x20LeagueProgressData=\"level=2\"\r\n\
\r\n\
\x20\x20\x20\x20\x20\x20[ResCore]\r\n\
\x20\x20\x20\x20\x20\x20Type=Player\r\n\
\x20\x20\x20\x20\x20\x20ID=23\r\n\
\x20\x20\x20\x20\x20\x20FileSize=123\r\n\
\x20\x20\x20\x20\x20\x20FileCRC=456\r\n\
\x20\x20\x20\x20\x20\x20ChunkSize=1024\r\n\
\x20\x20\x20\x20\x20\x20ContentsCRC=789\r\n\
\x20\x20\x20\x20\x20\x20FileSHA=000102030405060708090a0b0c0d0e0f10111213\r\n\
\x20\x20\x20\x20\x20\x20Filename=\"Players\\\\Alice.c4p\"\r\n\
\x20\x20\x20\x20\x20\x20Author=\"Host\"\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20ID=18\r\n\
\x20\x20\x20\x20Color=broken\r\n\
\x20\x20\x20\x20ExtraData=0000\r\n\
\x20\x20\x20\x20ProjectedGain=broken\r\n",
        )
        .expect("C4ClientPlayerInfos response parses");

        assert!(!response.head.is_success());
        assert_eq!(response.head.message.as_bytes(), b"Still parsed  ");
        assert_eq!(response.league.as_bytes(), b"Cup \x80");
        assert_eq!(response.player_infos.client_id, 9);
        assert_eq!(
            response.player_infos.flags,
            CLIENT_PLAYER_INFO_FLAG_UPDATED | CLIENT_PLAYER_INFO_FLAG_INITIAL
        );
        assert_eq!(response.player_infos.players.len(), 2);
        let alice = &response.player_infos.players[0];
        assert_eq!(alice.name.as_bytes(), b"Alice");
        assert_eq!(alice.forced_name.as_bytes(), b"Bob");
        assert_eq!(
            alice.flags,
            PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_HAS_RESOURCE | (1 << 6)
        );
        assert_eq!(
            (alice.id, alice.color, alice.original_color),
            (17, 1_193_046, 1_193_046)
        );
        assert_eq!((alice.game_number, alice.game_join_frame), (3, 44));
        assert_eq!(alice.league_projected_gain, -7);
        assert_eq!(alice.league_account.as_bytes(), b"Alice");
        assert_eq!(alice.clan_tag.as_bytes(), b"TAG");
        let resource = alice.resource.as_ref().expect("HasResource parses ResCore");
        assert_eq!((resource.resource_type, resource.id), (3, 23));
        assert_eq!((resource.file_size, resource.file_crc), (123, 456));
        assert_eq!(resource.chunk_size, 1024);
        assert_eq!(resource.contents_crc, 789);
        assert_eq!(
            resource.file_sha,
            Some(std::array::from_fn(|index| index as u8))
        );
        assert_eq!(resource.filename.as_bytes(), b"Players/Alice.c4p");
        assert_eq!(resource.author.as_bytes(), b"Host");

        let defaults = &response.player_infos.players[1];
        assert_eq!(defaults.id, 18);
        assert_eq!((defaults.color, defaults.original_color), (0, 0));
        assert_eq!(defaults.extra_data, *b"NONE");
        assert_eq!(defaults.league_projected_gain, -1);
        assert!(!defaults.league_progress_data_is_null);

        let reordered = decode_league_update_response(
            b"[Response]\r\n\r\n\x20\x20[PlayerInfos]\r\n\x20\x20ID=4\r\nStatus=Success\r\nMessage=After nested\r\n",
        )
        .expect("response fields remain children when written after a nested section");
        assert!(reordered.head.is_success());
        assert_eq!(reordered.head.message.as_bytes(), b"After nested");
        assert_eq!(reordered.player_infos.client_id, 4);
    }

    #[test]
    fn end_response_decodes_round_results_and_defaults_malformed_fields() {
        // GetEndReply validates only the common response Status. Each named
        // round-result scalar still carries its C++ naming default
        // (src/C4League.cpp:385-399; src/C4RoundResults.cpp:31-52).
        let response = decode_league_end_response(
            b"[Response]\r\n\
Status=Success\r\n\
Message=Scored  \r\n\
\r\n\
\x20\x20[PlayerInfos]\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20ID=17\r\n\
\x20\x20\x20\x20TotalPlayingTime=123\r\n\
\x20\x20\x20\x20SettlementScoreOld=40\r\n\
\x20\x20\x20\x20SettlementScoreNew=52\r\n\
\x20\x20\x20\x20Score=900\r\n\
\x20\x20\x20\x20GameScore=12\r\n\
\x20\x20\x20\x20Rank=3\r\n\
\x20\x20\x20\x20RankSymbol=4\r\n\
\x20\x20\x20\x20LeagueProgressData=\"done\\x21\"\r\n\
\x20\x20\x20\x20Status=Won\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20ID=18\r\n\
\x20\x20\x20\x20Status=Lost\r\n",
        )
        .expect("successful End response");

        assert_eq!(response.head.message.as_bytes(), b"Scored  ");
        assert_eq!(response.players.len(), 2);
        let winner = &response.players[0];
        assert_eq!(winner.player_info_id, 17);
        assert_eq!(winner.total_playing_time, 123);
        assert_eq!(
            (winner.settlement_score_old, winner.settlement_score_new),
            (40, 52)
        );
        assert_eq!(
            (winner.league_score_new, winner.league_score_gain),
            (900, 12)
        );
        assert_eq!(
            (winner.league_rank_new, winner.league_rank_symbol_new),
            (3, 4)
        );
        assert_eq!(winner.league_progress_data.as_bytes(), b"done!");
        assert_eq!(winner.status, LeagueRoundPlayerStatus::Won);
        let loser = &response.players[1];
        assert_eq!(loser.status, LeagueRoundPlayerStatus::Lost);
        assert_eq!(
            (loser.settlement_score_old, loser.league_score_gain),
            (-1, -1)
        );

        let defaulted = decode_league_end_response(
            b"[Response]\r\nStatus=Success\r\n\r\n\x20\x20[PlayerInfos]\r\n\r\n\x20\x20\x20\x20[Player]\r\n\x20\x20\x20\x20TotalPlayingTime=broken\r\n",
        )
        .expect("defaulted malformed field does not reject End");
        assert_eq!(defaulted.players.len(), 1);
        assert_eq!(defaulted.players[0].total_playing_time, 0);

        assert!(matches!(
            decode_league_end_response(
                b"[Response]\r\nStatus=Failure\r\nMessage=Rejected\r\n"
            ),
            Err(LeagueResponseDecodeError::EndRejected(response))
                if response.head.message.as_bytes() == b"Rejected"
        ));
    }

    #[test]
    fn successful_join_reply_applies_selected_league_data_and_consumes_auid() {
        // The host applies the account/clan and the score/rank/progress tuple
        // for Game.Parameters.League, then clears the one-use AUID after the
        // successful check (pristine 9ffa0a5d src/C4League.cpp:469-480;
        // src/C4Network2.cpp:2740-2776;
        // src/C4Network2Players.cpp:211-230).
        let response = decode_league_join_response(
            b"[Response]\r\n\
Status=Success\r\n\
Account= {<i>Alice</i>{ \r\n\
League=\"Other\",\"League\"\r\n\
Score=11,42\r\n\
Rank=1,7\r\n\
RankSymbol=2,9\r\n\
ProgressData=\"other\",\"level=3\"\r\n\
ClanTag= {<i>TAG</i>{ \r\n",
        );
        let mut player = clonk_engine::ControlPlayerInfoEntry {
            auth_id: legacy(b"one-use-token"),
            ..Default::default()
        };

        assert!(response.apply_auth_check(&legacy(b"League"), &mut player));
        assert!(player.auth_id.is_empty());
        assert_eq!(player.league_account.as_bytes(), b"Alice");
        assert_eq!(player.clan_tag.as_bytes(), b"TAG");
        assert_eq!(
            (
                player.league_score,
                player.league_rank,
                player.league_rank_symbol,
                player.league_progress_data.as_bytes(),
            ),
            (42, 7, 9, b"level=3".as_slice())
        );
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

    #[tokio::test]
    async fn league_http_post_preserves_cpp_headers_body_reply_and_status_errors() {
        // League Auth/Join pass the exact checksum-mutated INI bytes to Query
        // (pristine 9ffa0a5d src/C4League.cpp:401-420,451-466). A nonempty
        // Query is POSTed with these language/content headers, gzip support,
        // the engine user agent and HTTP failure handling
        // (src/C4Network2Reference.cpp:468-499;
        // src/C4HTTPClient.cpp:183-229).
        let body = b"[Request]\r\nAction=Auth\r\nChecksum=c0BAA\r\nName=P\x80\r\n";
        let reply = b"[Response]\r\nStatus=Success\r\nMessage=Welcome \x80\r\n";
        let (endpoint, request) = serve_one_http_response("200 OK", reply);
        let transport = LeagueHttpPostTransport::cpp_default().expect("build exact HTTP client");
        let config = LeagueHttpTransportConfig {
            language_charset: "RUSSIAN".to_owned(),
            language_sequence: "US,DE".to_owned(),
        };

        assert_eq!(
            transport
                .post(&endpoint, body, &config)
                .await
                .expect("C++-style league POST succeeds"),
            reply
        );

        let request = request.join().expect("local HTTP fixture exits");
        let (header, request_body) = split_http_request(&request);
        assert!(header.starts_with("POST /league?game=42 HTTP/1."));
        assert_header(header, "content-type", "text/plain; encoding=CP1251");
        assert_header(header, "accept-charset", "CP1251");
        assert_header(header, "accept-encoding", "gzip");
        assert_header(header, "accept-language", "US,DE");
        assert_header(header, "user-agent", "LegacyClonk/4.9.11.0 [362]");
        assert_header(header, "content-length", &body.len().to_string());
        assert!(!header.to_ascii_lowercase().contains("\r\nexpect:"));
        assert_eq!(request_body, body);

        let transport = LeagueHttpPostTransport::with_client(transport.client.clone());
        let (endpoint, request) = serve_one_http_response("503 League Offline", b"ignored");
        let error = transport
            .post(&endpoint, body, &config)
            .await
            .expect_err("C++ CURLOPT_FAILONERROR rejects HTTP failures");
        assert!(matches!(
            error,
            LeagueHttpTransportError::Request(ref source)
                if source.status() == Some(reqwest::StatusCode::SERVICE_UNAVAILABLE)
        ));
        let request = request.join().expect("error HTTP fixture exits");
        let (header, _) = split_http_request(&request);
        assert_header(header, "cookie", "LeagueSession=raw-byte-session");
    }

    fn serve_one_http_response(
        status: &'static str,
        body: &'static [u8],
    ) -> (String, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP fixture");
        let address = listener.local_addr().expect("fixture address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept league request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set fixture timeout");
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let count = stream.read(&mut buffer).expect("read league request");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let header =
                    std::str::from_utf8(&request[..header_end]).expect("request header is ASCII");
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("Content-Length")
                                .then(|| value.trim().parse::<usize>().expect("content length"))
                        })
                    })
                    .expect("request has Content-Length");
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nSet-Cookie: LeagueSession=raw-byte-session; Path=/\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write response header");
            stream.write_all(body).expect("write response body");
            request
        });
        (format!("http://{address}/league?game=42"), handle)
    }

    fn split_http_request(request: &[u8]) -> (&str, &[u8]) {
        let header_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .expect("complete request header");
        (
            std::str::from_utf8(&request[..header_end]).expect("request header is ASCII"),
            &request[header_end + 4..],
        )
    }

    fn assert_header(header: &str, expected_name: &str, expected_value: &str) {
        let value = header.lines().find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case(expected_name)
                    .then(|| value.trim())
            })
        });
        assert_eq!(value, Some(expected_value), "header {expected_name}");
    }

    #[test]
    fn insert_and_lookup_round_trip() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert(legacy(b"Alice"), legacy(b"FBID-123"));
        registry.insert(legacy(b"Bob"), legacy(b"FBID-456"));

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get(&legacy(b"Alice")), Some(&legacy(b"FBID-123")));
        assert_eq!(registry.get(&legacy(b"Bob")), Some(&legacy(b"FBID-456")));
        assert!(registry.get(&legacy(b"Eve")).is_none());
    }

    #[test]
    fn replacing_existing_account_overwrites_value() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert(legacy(b"A\x80"), legacy(b"FBID-\x81"));
        registry.insert(legacy(b"A\x80"), legacy(b"FBID-\xff"));

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(&legacy(b"A\x80")), Some(&legacy(b"FBID-\xff")));
    }

    #[test]
    fn removing_unknown_account_is_a_noop() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert(legacy(b"Alice"), legacy(b"FBID-123"));
        assert!(!registry.remove(&legacy(b"Bob")));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(&legacy(b"Alice")), Some(&legacy(b"FBID-123")));
    }

    #[test]
    fn removal_drops_entry() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert(legacy(b"Alice"), legacy(b"FBID-123"));
        registry.insert(legacy(b"Bob"), legacy(b"FBID-456"));

        assert!(registry.remove(&legacy(b"Alice")));
        assert_eq!(registry.get(&legacy(b"Alice")), None);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert(legacy(b"Alice"), legacy(b"FBID-123"));
        registry.insert(legacy(b"Bob"), legacy(b"FBID-456"));
        registry.clear();

        assert!(registry.is_empty());
        assert_eq!(registry.get(&legacy(b"Alice")), None);
        assert_eq!(registry.get(&legacy(b"Bob")), None);
    }
}
