use crate::DefinitionId;
use serde::{Deserialize, Serialize};

/// `C4RoundResults::NetResult`; `None` represents `NR_None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundResultsNetworkResult {
    LeagueOk,
    LeagueError,
    NetworkError,
}

/// C4RoundResultsPlayer::Status. The empty compiler token is represented by
/// `Unknown`; evaluated rows retain the exact Lost/Won distinction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundResultsPlayerStatus {
    #[default]
    Unknown,
    Lost,
    Won,
}

/// Per-player data retained by `C4RoundResultsPlayer` after evaluation.
///
/// The ID links to `C4PlayerInfo`, not the in-round `C4Player::Number`
/// (`C4RoundResults.h:36-46`). The two C++ score fields use `-1` as their
/// unknown sentinel (`C4RoundResults.h:63-69`); Rust keeps that sentinel for
/// the always-present old score and represents the optional new score
/// explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundResultsPlayerState {
    #[serde(default, skip_serializing_if = "is_unknown_player_status")]
    pub status: RoundResultsPlayerStatus,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub player_info_id: i32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub total_playing_time: u32,
    #[serde(default = "invalid_score", skip_serializing_if = "is_invalid_score")]
    pub score_old: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_new: Option<i32>,
    /// League score on the server after this round; `-1` is unknown.
    #[serde(default = "invalid_score", skip_serializing_if = "is_invalid_score")]
    pub league_score_new: i32,
    /// League score change awarded for this round. C++ constructs this as
    /// zero, while its serialized default for a missing field is `-1`.
    #[serde(default = "invalid_score", skip_serializing_if = "is_invalid_score")]
    pub league_score_gain: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub league_rank_new: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub league_rank_symbol_new: i32,
    /// Progress bytes copied from the linked C4PlayerInfo at evaluation time.
    /// This result-owned copy is persisted independently from later changes
    /// to the live player-info row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub league_progress_data: Option<Vec<u8>>,
    /// Scenario-defined league performance for this persistent player-info
    /// ID. C++ deliberately omits this temporary value from
    /// C4RoundResultsPlayer::CompileFunc, so serialized restores reset it.
    #[serde(skip)]
    pub league_performance: i32,
    /// Raw scenario-specific text consumed as one player-row label. C++
    /// appends multiple entries with exactly three spaces rather than a line
    /// separator (`C4RoundResults.cpp:94-98`).
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        with = "clonk_script::c4_string_serde"
    )]
    pub custom_evaluation_strings: String,
}

impl Default for RoundResultsPlayerState {
    fn default() -> Self {
        Self {
            status: RoundResultsPlayerStatus::Unknown,
            player_info_id: 0,
            total_playing_time: 0,
            score_old: invalid_score(),
            score_new: None,
            league_score_new: invalid_score(),
            league_score_gain: 0,
            league_rank_new: 0,
            league_rank_symbol_new: 0,
            league_progress_data: None,
            league_performance: 0,
            custom_evaluation_strings: String::new(),
        }
    }
}

/// Simulation-owned subset of `C4RoundResults` required by the classic
/// evaluation dialog (`C4RoundResults.h:118-138`). Evaluation remains a
/// separate behavioral step; this type only makes the resulting state
/// snapshot- and save-safe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundResultsState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<DefinitionId>,
    /// Exact ordered C4IDList backing `Goals`, including signed and zero
    /// counts. `goals` remains the unique behavioral projection consumed by
    /// the evaluation UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goal_counts: Vec<(DefinitionId, i32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fulfilled_goals: Vec<DefinitionId>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub playing_time_seconds: u32,
    /// `C4RoundResults::fHideSettlementScore`; the scenario/melee decision
    /// that sets it remains part of the later behavioral slice
    /// (`C4RoundResults.cpp:240-245,362-369`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_settlement_score: bool,
    /// Global scenario-defined league performance. Unlike the per-player
    /// value, C4RoundResults::CompileFunc serializes this field.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub league_performance: i32,
    /// Raw global evaluation text. C++ concatenates entries with `|`, which
    /// its multiline GUI interprets as line separators
    /// (`C4RoundResults.cpp:346-354`).
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        with = "clonk_script::c4_string_serde"
    )]
    pub custom_evaluation_strings: String,
    /// First network evaluation applied to these results. C++ preserves an
    /// earlier, usually more-specific disconnect error over a league reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_result: Option<RoundResultsNetworkResult>,
    /// Raw legacy result text paired with `network_result`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_result_message: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<RoundResultsPlayerState>,
}

impl RoundResultsState {
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
            && self.goal_counts.is_empty()
            && self.fulfilled_goals.is_empty()
            && self.playing_time_seconds == 0
            && !self.hide_settlement_score
            && self.league_performance == 0
            && self.custom_evaluation_strings.is_empty()
            && self.network_result.is_none()
            && self.network_result_message.is_empty()
            && self.players.is_empty()
    }

    /// `C4RoundResults::AddCustomEvaluationString` and its per-player
    /// counterpart (C4RoundResults.cpp:94-98,346-359). Global entries are
    /// separate GUI lines (`|`); entries in one player row use exactly three
    /// spaces. `FnAddEvaluationData` validates the text and player-info ID
    /// before reaching this state mutation.
    pub fn add_custom_evaluation_string(&mut self, text: &str, player_info_id: i32) {
        if player_info_id == 0 {
            if !self.custom_evaluation_strings.is_empty() {
                self.custom_evaluation_strings.push('|');
            }
            self.custom_evaluation_strings.push_str(text);
            return;
        }

        let index = self
            .players
            .iter()
            .position(|player| player.player_info_id == player_info_id)
            .unwrap_or_else(|| {
                let index = self.players.len();
                self.players.push(RoundResultsPlayerState {
                    player_info_id,
                    ..RoundResultsPlayerState::default()
                });
                index
            });
        let player = &mut self.players[index];
        if !player.custom_evaluation_strings.is_empty() {
            player.custom_evaluation_strings.push_str("   ");
        }
        player.custom_evaluation_strings.push_str(text);
    }

    /// `C4RoundResults::SetLeaguePerformance`: zero selects the independent
    /// global slot; nonzero IDs overwrite or append an exact player-info row.
    pub fn set_league_performance(&mut self, score: i32, player_info_id: i32) {
        if player_info_id == 0 {
            self.league_performance = score;
            return;
        }
        let index = self
            .players
            .iter()
            .position(|player| player.player_info_id == player_info_id)
            .unwrap_or_else(|| {
                let index = self.players.len();
                self.players.push(RoundResultsPlayerState {
                    player_info_id,
                    ..RoundResultsPlayerState::default()
                });
                index
            });
        self.players[index].league_performance = score;
    }

    /// `C4RoundResults::EvaluateNetwork`: the first result is usually the most
    /// specific, so later disconnect/league outcomes cannot replace it.
    pub fn evaluate_network(
        &mut self,
        result: RoundResultsNetworkResult,
        result_message: Option<Vec<u8>>,
    ) {
        if self.network_result.is_none() {
            self.network_result = Some(result);
            self.network_result_message = result_message.unwrap_or_default();
        }
    }

    /// Applies `C4RoundResults::EvaluateLeague`: retain the first network
    /// result and copy only the server-owned league fields into rows keyed by
    /// persistent player-info ID. Local settlement score and playing time are
    /// deliberately untouched.
    pub fn evaluate_league(
        &mut self,
        success: bool,
        result_message: Vec<u8>,
        players: impl IntoIterator<Item = LeagueRoundResultUpdate>,
    ) {
        self.evaluate_network(
            if success {
                RoundResultsNetworkResult::LeagueOk
            } else {
                RoundResultsNetworkResult::LeagueError
            },
            Some(result_message),
        );
        for update in players {
            let index = self
                .players
                .iter()
                .position(|player| player.player_info_id == update.player_info_id)
                .unwrap_or_else(|| {
                    let index = self.players.len();
                    self.players.push(RoundResultsPlayerState {
                        player_info_id: update.player_info_id,
                        ..RoundResultsPlayerState::default()
                    });
                    index
                });
            let player = &mut self.players[index];
            player.league_score_new = update.league_score_new;
            player.league_score_gain = update.league_score_gain;
            player.league_rank_new = update.league_rank_new;
            player.league_rank_symbol_new = update.league_rank_symbol_new;
            player.league_progress_data = Some(update.league_progress_data);
        }
    }

    /// C4RoundResultsPlayer::CompileFunc omits its temporary performance
    /// field. Save-state construction must therefore clear it even when the
    /// in-memory state is restored directly without a JSON round trip.
    pub(crate) fn prepare_for_save(&mut self) {
        for player in &mut self.players {
            player.league_performance = 0;
            if player
                .league_progress_data
                .as_ref()
                .is_some_and(Vec::is_empty)
            {
                player.league_progress_data = None;
            }
        }
    }

    /// Compiles the native `RoundResults.txt` component. The two
    /// `NetResult` namings are intentionally consumed in file order: C++
    /// first reads the result text and then the enum through two identically
    /// named adapters (`C4RoundResults.cpp:259-278`).
    pub(crate) fn from_legacy_ini(source: &[u8], melee: bool) -> Result<Self, String> {
        if source.is_empty() {
            return Err("empty group entry".to_owned());
        }

        let tree = LegacyRoundResultsIni::parse(source);
        let Some(root) = tree.first_child(0, b"RoundResults", 0) else {
            return Err("missing [RoundResults] section".to_owned());
        };

        let goal_counts = tree
            .value(root, b"Goals", 0)
            .map(parse_goal_list)
            .unwrap_or_default();
        let mut goals = Vec::new();
        for (id, _) in &goal_counts {
            if !goals.contains(id) {
                goals.push(id.clone());
            }
        }

        let players = match tree.first_child(root, b"PlayerInfos", 0) {
            Some(player_infos) => {
                let player_nodes = tree.children(player_infos, b"Player");
                if player_nodes.len() > 5_000 {
                    return Err(format!(
                        "player count out of range: {}",
                        player_nodes.len()
                    ));
                }
                player_nodes
                    .into_iter()
                    .map(|node| parse_player_result(&tree, node))
                    .collect()
            }
            None => Vec::new(),
        };

        let network_result_message = tree
            .value(root, b"NetResult", 0)
            .map(parse_legacy_string)
            .unwrap_or_default();
        let network_result = tree
            .value(root, b"NetResult", 1)
            .and_then(parse_network_result);

        Ok(Self {
            goals,
            goal_counts,
            // FulfilledGoals is deliberately absent from CompileFunc.
            fulfilled_goals: Vec::new(),
            playing_time_seconds: tree
                .value(root, b"PlayingTime", 0)
                .and_then(parse_u32)
                .unwrap_or(0),
            hide_settlement_score: tree
                .value(root, b"HideSettlementScore", 0)
                .and_then(parse_bool)
                .unwrap_or(melee),
            league_performance: tree
                .value(root, b"LeaguePerformance", 0)
                .and_then(parse_i32)
                .unwrap_or(0),
            custom_evaluation_strings: tree
                .value(root, b"CustomEvaluationStrings", 0)
                .map(parse_legacy_c4_string)
                .unwrap_or_default(),
            network_result,
            network_result_message,
            players,
        })
    }
}

#[derive(Debug)]
struct LegacyRoundResultsIniNode {
    name: Vec<u8>,
    value: Vec<u8>,
    indent: isize,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// The small subset of `StdCompilerINIRead`'s name tree needed by
/// C4RoundResults. Names are case-sensitive and repeated names remain in
/// insertion order, including the duplicate `NetResult` fields.
#[derive(Debug)]
struct LegacyRoundResultsIni {
    nodes: Vec<LegacyRoundResultsIniNode>,
}

impl LegacyRoundResultsIni {
    fn parse(source: &[u8]) -> Self {
        let source = &source[..source.iter().position(|byte| *byte == 0).unwrap_or(source.len())];
        let mut tree = Self {
            nodes: vec![LegacyRoundResultsIniNode {
                name: Vec::new(),
                value: Vec::new(),
                indent: -1,
                parent: None,
                children: Vec::new(),
            }],
        };
        let mut current = 0;
        let mut position = 0;
        while position < source.len() {
            let line_end = source[position..]
                .iter()
                .position(|byte| matches!(*byte, b'\r' | b'\n'))
                .map(|offset| position + offset)
                .unwrap_or(source.len());
            let line = &source[position..line_end];
            position = line_end;
            while source
                .get(position)
                .is_some_and(|byte| matches!(*byte, b'\r' | b'\n'))
            {
                position += 1;
            }

            let indent = line
                .iter()
                .take_while(|byte| matches!(**byte, b' ' | b'\t'))
                .count();
            let mut cursor = indent;
            let section = line.get(cursor) == Some(&b'[')
                && line
                    .get(cursor + 1)
                    .is_some_and(u8::is_ascii_alphabetic);
            if section {
                cursor += 1;
            } else if !line.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            let name_start = cursor;
            while line.get(cursor).is_some_and(|byte| {
                byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_')
            }) {
                cursor += 1;
            }
            let name = &line[name_start..cursor];
            while line
                .get(cursor)
                .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            let separator = if section { b']' } else { b'=' };
            if line.get(cursor) != Some(&separator) {
                continue;
            }
            cursor += 1;

            let node_indent = (indent + usize::from(!section)) as isize;
            while current != 0 && tree.nodes[current].indent >= node_indent {
                current = tree.nodes[current].parent.unwrap_or(0);
            }
            let index = tree.nodes.len();
            tree.nodes.push(LegacyRoundResultsIniNode {
                name: name.to_vec(),
                value: (!section).then(|| line[cursor..].to_vec()).unwrap_or_default(),
                indent: node_indent,
                parent: Some(current),
                children: Vec::new(),
            });
            tree.nodes[current].children.push(index);
            if section {
                current = index;
            }
        }
        tree
    }

    fn first_child(&self, parent: usize, name: &[u8], occurrence: usize) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .filter(|index| self.nodes[*index].name == name)
            .nth(occurrence)
    }

    fn children(&self, parent: usize, name: &[u8]) -> Vec<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .filter(|index| self.nodes[*index].name == name)
            .collect()
    }

    fn value(&self, parent: usize, name: &[u8], occurrence: usize) -> Option<&[u8]> {
        self.first_child(parent, name, occurrence)
            .map(|index| self.nodes[index].value.as_slice())
    }
}

fn parse_player_result(tree: &LegacyRoundResultsIni, node: usize) -> RoundResultsPlayerState {
    let score_new = tree
        .value(node, b"SettlementScoreNew", 0)
        .and_then(parse_i32)
        .unwrap_or(-1);
    RoundResultsPlayerState {
        status: tree
            .value(node, b"Status", 0)
            .map(parse_player_status)
            .unwrap_or_default(),
        player_info_id: tree
            .value(node, b"ID", 0)
            .and_then(parse_i32)
            .unwrap_or(0),
        total_playing_time: tree
            .value(node, b"TotalPlayingTime", 0)
            .and_then(parse_u32)
            .unwrap_or(0),
        score_old: tree
            .value(node, b"SettlementScoreOld", 0)
            .and_then(parse_i32)
            .unwrap_or(-1),
        score_new: (score_new != -1).then_some(score_new),
        league_score_new: tree
            .value(node, b"Score", 0)
            .and_then(parse_i32)
            .unwrap_or(-1),
        // CompileFunc's missing-field default differs from the constructor.
        league_score_gain: tree
            .value(node, b"GameScore", 0)
            .and_then(parse_i32)
            .unwrap_or(-1),
        league_rank_new: tree
            .value(node, b"Rank", 0)
            .and_then(parse_i32)
            .unwrap_or(0),
        league_rank_symbol_new: tree
            .value(node, b"RankSymbol", 0)
            .and_then(parse_i32)
            .unwrap_or(0),
        league_progress_data: tree
            .value(node, b"LeagueProgressData", 0)
            .map(parse_legacy_string),
        league_performance: 0,
        // This field is intentionally not part of C4RoundResultsPlayer::CompileFunc.
        custom_evaluation_strings: String::new(),
    }
}

fn parse_goal_list(raw: &[u8]) -> Vec<(DefinitionId, i32)> {
    let mut entries = Vec::new();
    let mut position = 0;
    let mut first = true;
    loop {
        if !first && !consume_separator(raw, &mut position, b';') {
            break;
        }
        first = false;
        skip_horizontal_whitespace(raw, &mut position);
        let start = position;
        while position < raw.len()
            && position - start < 4
            && (raw[position].is_ascii_alphanumeric()
                || matches!(raw[position], b'_' | b'-'))
        {
            position += 1;
        }
        let id = &raw[start..position];
        if !valid_c4_id(id) {
            break;
        }
        let count = if consume_separator(raw, &mut position, b'=') {
            parse_i32_at(raw, &mut position).unwrap_or(0)
        } else {
            0
        };
        entries.push((String::from_utf8_lossy(id).into_owned(), count));
    }
    entries
}

fn valid_c4_id(id: &[u8]) -> bool {
    if id.len() != 4 || id == b"NONE" {
        return false;
    }
    if id.iter().all(u8::is_ascii_digit) {
        return std::str::from_utf8(id)
            .ok()
            .and_then(|id| id.parse::<u16>().ok())
            .is_some_and(|id| id != 0);
    }
    id.iter()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn parse_i32(raw: &[u8]) -> Option<i32> {
    let mut position = 0;
    parse_i32_at(raw, &mut position)
}

fn parse_i32_at(raw: &[u8], position: &mut usize) -> Option<i32> {
    skip_horizontal_whitespace(raw, position);
    let start = *position;
    let signed = matches!(raw.get(start), Some(b'+' | b'-'));
    let sign_length = usize::from(signed);
    let unsigned_start = start + sign_length;
    let hexadecimal = !signed
        && raw.get(unsigned_start) == Some(&b'0')
        && matches!(raw.get(unsigned_start + 1), Some(b'x' | b'X'));
    let digit_start = unsigned_start + usize::from(hexadecimal) * 2;
    let digit_length = raw.get(digit_start..)?.iter().take_while(|byte| {
        if hexadecimal {
            byte.is_ascii_hexdigit()
        } else {
            byte.is_ascii_digit()
        }
    }).count();
    if digit_length == 0 {
        return None;
    }
    let end = digit_start + digit_length;
    *position = end;
    let digits = std::str::from_utf8(&raw[digit_start..end]).ok()?;
    let magnitude = i64::from_str_radix(digits, if hexadecimal { 16 } else { 10 }).ok()?;
    let value = if raw.get(start) == Some(&b'-') {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    i32::try_from(value).ok()
}

fn parse_u32(raw: &[u8]) -> Option<u32> {
    let mut position = 0;
    skip_horizontal_whitespace(raw, &mut position);
    let negative = raw.get(position) == Some(&b'-');
    let positive = raw.get(position) == Some(&b'+');
    if negative || positive {
        position += 1;
    }
    let hexadecimal = !negative
        && !positive
        && raw.get(position) == Some(&b'0')
        && matches!(raw.get(position + 1), Some(b'x' | b'X'));
    if hexadecimal {
        position += 2;
    }
    let start = position;
    while raw.get(position).is_some_and(|byte| {
        if hexadecimal {
            byte.is_ascii_hexdigit()
        } else {
            byte.is_ascii_digit()
        }
    }) {
        position += 1;
    }
    if position == start {
        return None;
    }
    let digits = std::str::from_utf8(&raw[start..position]).ok()?;
    let magnitude = u64::from_str_radix(digits, if hexadecimal { 16 } else { 10 }).ok()?;
    Some(if negative {
        0_u64.wrapping_sub(magnitude) as u32
    } else {
        magnitude as u32
    })
}

fn parse_bool(raw: &[u8]) -> Option<bool> {
    if raw.starts_with(b"1") && !raw.get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if raw.starts_with(b"0") && !raw.get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if raw.starts_with(b"true") {
        Some(true)
    } else if raw.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

fn parse_network_result(raw: &[u8]) -> Option<RoundResultsNetworkResult> {
    if let Some(value) = parse_i32(raw) {
        return match value.clamp(0, u8::MAX.into()) as u8 {
            1 => Some(RoundResultsNetworkResult::LeagueOk),
            2 => Some(RoundResultsNetworkResult::LeagueError),
            3 => Some(RoundResultsNetworkResult::NetworkError),
            _ => None,
        };
    }
    match parse_enum_token(raw) {
        b"LeagueOK" => Some(RoundResultsNetworkResult::LeagueOk),
        b"LeagueError" => Some(RoundResultsNetworkResult::LeagueError),
        b"NetError" => Some(RoundResultsNetworkResult::NetworkError),
        _ => None,
    }
}

fn parse_player_status(raw: &[u8]) -> RoundResultsPlayerStatus {
    if let Some(value) = parse_i32(raw) {
        return match value.clamp(0, u8::MAX.into()) as u8 {
            1 => RoundResultsPlayerStatus::Lost,
            2 => RoundResultsPlayerStatus::Won,
            _ => RoundResultsPlayerStatus::Unknown,
        };
    }
    match parse_enum_token(raw) {
        b"Lost" => RoundResultsPlayerStatus::Lost,
        b"Won" => RoundResultsPlayerStatus::Won,
        _ => RoundResultsPlayerStatus::Unknown,
    }
}

fn parse_enum_token(raw: &[u8]) -> &[u8] {
    let start = raw
        .iter()
        .position(|byte| !matches!(*byte, b' ' | b'\t'))
        .unwrap_or(raw.len());
    let raw = &raw[start..];
    let length = raw
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(**byte, b'_' | b'-'))
        .count();
    &raw[..length]
}

fn parse_legacy_c4_string(raw: &[u8]) -> String {
    clonk_script::c4_string_from_bytes(&parse_legacy_string(raw))
}

fn parse_legacy_string(raw: &[u8]) -> Vec<u8> {
    // StdCompilerINIRead decides escaped-vs-RCT_All before it skips leading
    // spaces, so `Field= "text"` is an unescaped string containing quotes.
    if !raw.starts_with(b"\"") {
        let start = raw
            .iter()
            .position(|byte| !matches!(*byte, b' ' | b'\t'))
            .unwrap_or(raw.len());
        return raw[start..].to_vec();
    }

    let mut output = Vec::new();
    let mut position = 1;
    while let Some(&byte) = raw.get(position) {
        if byte == b'\"' {
            break;
        }
        if byte != b'\\' {
            output.push(byte);
            position += 1;
            continue;
        }
        position += 1;
        let Some(&escaped) = raw.get(position) else {
            break;
        };
        position += 1;
        match escaped {
            b'a' => output.push(0x07),
            b'b' => output.push(0x08),
            b'f' => output.push(0x0c),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(0x0b),
            b'\'' => output.push(b'\''),
            b'\"' => output.push(b'\"'),
            b'\\' => output.push(b'\\'),
            b'?' => output.push(b'?'),
            b'x' => {
                let start = position;
                while raw.get(position).is_some_and(u8::is_ascii_hexdigit) {
                    position += 1;
                }
                if position == start {
                    output.push(b'x');
                } else {
                    let mut value = 0_u8;
                    for byte in &raw[start..position] {
                        value = value.wrapping_mul(16).wrapping_add(match byte {
                            b'0'..=b'9' => *byte - b'0',
                            b'a'..=b'f' => *byte - b'a' + 10,
                            b'A'..=b'F' => *byte - b'A' + 10,
                            _ => 0,
                        });
                    }
                    output.push(value);
                }
            }
            b'0'..=b'7' => {
                let mut value = escaped - b'0';
                while let Some(byte @ b'0'..=b'7') = raw.get(position).copied() {
                    value = value.wrapping_mul(8).wrapping_add(byte - b'0');
                    position += 1;
                }
                output.push(value);
            }
            other => output.push(other),
        }
    }
    if let Some(nul) = output.iter().position(|byte| *byte == 0) {
        output.truncate(nul);
    }
    output
}

fn consume_separator(raw: &[u8], position: &mut usize, separator: u8) -> bool {
    skip_horizontal_whitespace(raw, position);
    if raw.get(*position) != Some(&separator) {
        return false;
    }
    *position += 1;
    true
}

fn skip_horizontal_whitespace(raw: &[u8], position: &mut usize) {
    while raw
        .get(*position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        *position += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueRoundResultUpdate {
    pub player_info_id: i32,
    pub league_score_new: i32,
    pub league_score_gain: i32,
    pub league_rank_new: i32,
    pub league_rank_symbol_new: i32,
    pub league_progress_data: Vec<u8>,
}

const fn invalid_score() -> i32 {
    -1
}

fn is_invalid_score(value: &i32) -> bool {
    *value == invalid_score()
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_unknown_player_status(value: &RoundResultsPlayerStatus) -> bool {
    *value == RoundResultsPlayerStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_defaults_match_cpp_unknown_score_sentinels() {
        let player = RoundResultsPlayerState::default();
        assert_eq!(player.player_info_id, 0);
        assert_eq!(player.total_playing_time, 0);
        assert_eq!(player.score_old, -1);
        assert_eq!(player.score_new, None);
        assert_eq!(player.league_score_new, -1);
        assert_eq!(player.league_score_gain, 0);
        assert_eq!(player.league_performance, 0);
        assert!(player.custom_evaluation_strings.is_empty());

        let mut decoded: RoundResultsPlayerState = serde_json::from_str("{}")
            .unwrap_or_else(|error| panic!("default player result parses: {error}"));
        // CompileFunc defaults a missing GameScore to -1 even though the C++
        // constructor initializes a fresh in-memory result to zero.
        assert_eq!(decoded.league_score_gain, -1);
        decoded.league_score_gain = 0;
        assert_eq!(decoded, player);

        let encoded = serde_json::to_value(&player)
            .unwrap_or_else(|error| panic!("fresh player result serializes: {error}"));
        assert_eq!(encoded.get("league_score_gain"), Some(&serde_json::json!(0)));
    }

    #[test]
    fn league_evaluation_preserves_local_settlement_fields_and_persists_server_fields() {
        let mut results = RoundResultsState {
            players: vec![RoundResultsPlayerState {
                player_info_id: 7,
                total_playing_time: 90,
                score_old: 10,
                score_new: Some(12),
                ..RoundResultsPlayerState::default()
            }],
            ..RoundResultsState::default()
        };
        results.evaluate_league(
            true,
            b"evaluated".to_vec(),
            [LeagueRoundResultUpdate {
                player_info_id: 7,
                league_score_new: 80,
                league_score_gain: -5,
                league_rank_new: 3,
                league_rank_symbol_new: 4,
                league_progress_data: b"progress".to_vec(),
            }],
        );

        assert_eq!(
            results.network_result,
            Some(RoundResultsNetworkResult::LeagueOk)
        );
        assert_eq!(results.network_result_message, b"evaluated");
        let player = &results.players[0];
        assert_eq!((player.total_playing_time, player.score_old, player.score_new), (90, 10, Some(12)));
        assert_eq!(
            (
                player.league_score_new,
                player.league_score_gain,
                player.league_rank_new,
                player.league_rank_symbol_new,
            ),
            (80, -5, 3, 4)
        );
        assert_eq!(player.league_progress_data.as_deref(), Some(&b"progress"[..]));
    }

    #[test]
    fn league_evaluation_retains_first_result_and_updates_server_owned_player_fields() {
        let mut results = RoundResultsState {
            players: vec![RoundResultsPlayerState {
                player_info_id: 7,
                total_playing_time: 90,
                score_old: 10,
                score_new: Some(12),
                ..RoundResultsPlayerState::default()
            }],
            ..RoundResultsState::default()
        };
        let update = |player_info_id, league_score_new| LeagueRoundResultUpdate {
            player_info_id,
            league_score_new,
            league_score_gain: -5,
            league_rank_new: 3,
            league_rank_symbol_new: 4,
            league_progress_data: b"progress".to_vec(),
        };

        results.evaluate_network(
            RoundResultsNetworkResult::NetworkError,
            Some(b"first".to_vec()),
        );
        results.evaluate_league(true, b"second".to_vec(), [update(7, 81), update(9, 70)]);

        assert_eq!(
            results.network_result,
            Some(RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            results.network_result_message.as_slice(),
            &b"first"[..]
        );
        let retained = results
            .players
            .iter()
            .find(|player| player.player_info_id == 7)
            .unwrap();
        assert_eq!(
            (retained.total_playing_time, retained.score_old, retained.score_new),
            (90, 10, Some(12))
        );
        assert_eq!(
            (
                retained.league_score_new,
                retained.league_score_gain,
                retained.league_rank_new,
                retained.league_rank_symbol_new,
                retained.league_progress_data.as_deref(),
            ),
            (81, -5, 3, 4, Some(&b"progress"[..]))
        );
        let created = results
            .players
            .iter()
            .find(|player| player.player_info_id == 9)
            .unwrap();
        assert_eq!((created.score_old, created.score_new), (-1, None));
        assert_eq!(created.league_score_new, 70);

        let serialized = serde_json::to_string(&results).unwrap();
        assert_eq!(
            serde_json::from_str::<RoundResultsState>(&serialized).unwrap(),
            results
        );

        let mut league_error = RoundResultsState::default();
        league_error.evaluate_league(false, b"backend error".to_vec(), []);
        assert_eq!(
            league_error.network_result,
            Some(RoundResultsNetworkResult::LeagueError)
        );
        assert_eq!(
            league_error.network_result_message.as_slice(),
            &b"backend error"[..]
        );
    }

    #[test]
    fn dialog_metadata_keeps_nondefault_results_nonempty() {
        assert!(
            !RoundResultsState {
                hide_settlement_score: true,
                ..RoundResultsState::default()
            }
            .is_empty()
        );
        assert!(
            !RoundResultsState {
                league_performance: -1,
                ..RoundResultsState::default()
            }
            .is_empty()
        );
        assert!(
            !RoundResultsState {
                custom_evaluation_strings: "First|Second".to_string(),
                ..RoundResultsState::default()
            }
            .is_empty()
        );
        assert!(
            !RoundResultsState {
                players: vec![RoundResultsPlayerState {
                    custom_evaluation_strings: "First   Second".to_string(),
                    ..RoundResultsPlayerState::default()
                }],
                ..RoundResultsState::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn custom_evaluation_strings_keep_cpp_global_and_player_separators() {
        let mut results = RoundResultsState::default();
        results.add_custom_evaluation_string("Global one", 0);
        results.add_custom_evaluation_string("Global two", 0);
        results.add_custom_evaluation_string("Kills: 3", 17);
        results.add_custom_evaluation_string("Deaths: 1", 17);
        results.add_custom_evaluation_string("Other", 9);

        assert_eq!(results.custom_evaluation_strings, "Global one|Global two");
        assert_eq!(
            results
                .players
                .iter()
                .map(|player| (
                    player.player_info_id,
                    player.custom_evaluation_strings.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![(17, "Kills: 3   Deaths: 1"), (9, "Other")]
        );
    }

    #[test]
    fn live_serializer_round_results_output_compiles_back_to_runtime_state() {
        // Exact output shape of live_c4_save::serialize_round_results,
        // including nested naming indentation and the two NetResult fields.
        // `concat!` preserves the significant INI indentation. A Rust string
        // line continuation would strip the leading spaces and flatten the
        // two nested native naming sections.
        let source = concat!(
            "[RoundResults]\r\n",
            "Goals=ZERO=0;DEBT=-2\r\n",
            "PlayingTime=123\r\n",
            "HideSettlementScore=true\r\n",
            "CustomEvaluationStrings=\"first|second\\n\\200\"\r\n",
            "LeaguePerformance=-7\r\n\r\n",
            "  [PlayerInfos]\r\n\r\n",
            "    [Player]\r\n",
            "    ID=7\r\n",
            "    TotalPlayingTime=90\r\n",
            "    SettlementScoreOld=10\r\n",
            "    SettlementScoreNew=12\r\n",
            "    Score=80\r\n",
            "    GameScore=-5\r\n",
            "    Rank=3\r\n",
            "    RankSymbol=4\r\n",
            "    LeagueProgressData=\"p\\\\\\\"\\r\\n\\201\"\r\n",
            "    Status=Won\r\n",
            "NetResult=\"bad \\\\\\\"line\\n\\200\"\r\n",
            "NetResult=LeagueError\r\n",
        )
        .as_bytes();

        let restored = RoundResultsState::from_legacy_ini(source, false)
            .expect("live RoundResults.txt compiles");
        assert_eq!(
            restored,
            RoundResultsState {
                goals: vec!["ZERO".to_owned(), "DEBT".to_owned()],
                goal_counts: vec![("ZERO".to_owned(), 0), ("DEBT".to_owned(), -2)],
                fulfilled_goals: Vec::new(),
                playing_time_seconds: 123,
                hide_settlement_score: true,
                league_performance: -7,
                custom_evaluation_strings: clonk_script::c4_string_from_bytes(
                    b"first|second\n\x80"
                ),
                network_result: Some(RoundResultsNetworkResult::LeagueError),
                network_result_message: b"bad \\\"line\n\x80".to_vec(),
                players: vec![RoundResultsPlayerState {
                    status: RoundResultsPlayerStatus::Won,
                    player_info_id: 7,
                    total_playing_time: 90,
                    score_old: 10,
                    score_new: Some(12),
                    league_score_new: 80,
                    league_score_gain: -5,
                    league_rank_new: 3,
                    league_rank_symbol_new: 4,
                    league_progress_data: Some(b"p\\\"\r\n\x81".to_vec()),
                    league_performance: 0,
                    custom_evaluation_strings: String::new(),
                }],
            }
        );
    }

    #[test]
    fn legacy_round_results_defaults_match_compilefunc_and_melee_init() {
        let melee = RoundResultsState::from_legacy_ini(b"[RoundResults]\r\n", true)
            .expect("empty named block uses defaults");
        assert!(melee.hide_settlement_score);
        assert!(melee.players.is_empty());

        let player = RoundResultsState::from_legacy_ini(
            b"[RoundResults]\r\n\r\n  [PlayerInfos]\r\n\r\n    [Player]\r\n    ID=4\r\n",
            false,
        )
        .expect("default player compiles")
        .players
        .pop()
        .expect("one named player");
        assert_eq!(player.league_score_gain, -1);
        assert_eq!(player.score_new, None);
        assert_eq!(player.league_progress_data, None);
    }

    #[test]
    fn duplicate_net_result_name_is_consumed_in_cpp_field_order() {
        let two = RoundResultsState::from_legacy_ini(
            b"[RoundResults]\r\nNetResult=\"detail\"\r\nNetResult=NetError\r\n",
            false,
        )
        .expect("two fields compile");
        assert_eq!(two.network_result_message, b"detail");
        assert_eq!(
            two.network_result,
            Some(RoundResultsNetworkResult::NetworkError)
        );

        // With only one naming, C++'s first StdStrBuf adapter consumes it;
        // the later enum adapter sees a missing field and takes NR_None.
        let one = RoundResultsState::from_legacy_ini(
            b"[RoundResults]\r\nNetResult=LeagueOK\r\n",
            false,
        )
        .expect("single field compiles as result text");
        assert_eq!(one.network_result_message, b"LeagueOK");
        assert_eq!(one.network_result, None);
    }

    #[test]
    fn malformed_or_empty_round_results_component_fails_load() {
        assert!(RoundResultsState::from_legacy_ini(b"", false).is_err());
        assert!(RoundResultsState::from_legacy_ini(b"[Other]\r\n", false).is_err());
    }
}
