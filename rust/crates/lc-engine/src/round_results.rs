use crate::DefinitionId;
use serde::{Deserialize, Serialize};

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
    /// Scenario-defined league performance for this persistent player-info
    /// ID. C++ deliberately omits this temporary value from
    /// C4RoundResultsPlayer::CompileFunc, so serialized restores reset it.
    #[serde(skip)]
    pub league_performance: i32,
    /// Raw scenario-specific text consumed as one player-row label. C++
    /// appends multiple entries with exactly three spaces rather than a line
    /// separator (`C4RoundResults.cpp:94-98`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_evaluation_strings: String,
}

impl Default for RoundResultsPlayerState {
    fn default() -> Self {
        Self {
            player_info_id: 0,
            total_playing_time: 0,
            score_old: invalid_score(),
            score_new: None,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_evaluation_strings: String,
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

    /// C4RoundResultsPlayer::CompileFunc omits its temporary performance
    /// field. Save-state construction must therefore clear it even when the
    /// in-memory state is restored directly without a JSON round trip.
    pub(crate) fn prepare_for_save(&mut self) {
        for player in &mut self.players {
            player.league_performance = 0;
        }
    }
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
        assert_eq!(player.league_performance, 0);
        assert!(player.custom_evaluation_strings.is_empty());

        let decoded: RoundResultsPlayerState = serde_json::from_str("{}")
            .unwrap_or_else(|error| panic!("default player result parses: {error}"));
        assert_eq!(decoded, player);
    }

    #[test]
    fn dialog_metadata_keeps_nondefault_results_nonempty() {
        assert!(!RoundResultsState {
            hide_settlement_score: true,
            ..RoundResultsState::default()
        }
        .is_empty());
        assert!(!RoundResultsState {
            league_performance: -1,
            ..RoundResultsState::default()
        }
        .is_empty());
        assert!(!RoundResultsState {
            custom_evaluation_strings: "First|Second".to_string(),
            ..RoundResultsState::default()
        }
        .is_empty());
        assert!(!RoundResultsState {
            players: vec![RoundResultsPlayerState {
                custom_evaluation_strings: "First   Second".to_string(),
                ..RoundResultsPlayerState::default()
            }],
            ..RoundResultsState::default()
        }
        .is_empty());
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
                .map(|player| (player.player_info_id, player.custom_evaluation_strings.as_str()))
                .collect::<Vec<_>>(),
            vec![(17, "Kills: 3   Deaths: 1"), (9, "Other")]
        );
    }
}
