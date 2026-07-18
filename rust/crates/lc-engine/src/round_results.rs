use crate::DefinitionId;
use serde::{Deserialize, Serialize};

/// `C4RoundResults::NetResult`; `None` represents `NR_None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundResultsNetworkResult {
    LeagueOk,
    LeagueError,
    NetworkError,
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
        with = "lc_script::c4_string_serde"
    )]
    pub custom_evaluation_strings: String,
}

impl Default for RoundResultsPlayerState {
    fn default() -> Self {
        Self {
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
        with = "lc_script::c4_string_serde"
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
}
