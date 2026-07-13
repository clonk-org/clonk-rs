use serde::{Deserialize, Serialize};

/// The key shared by the caption row and caption column
/// (`C4Scoreboard::TitleKey`, C4Scoreboard.h:29).
pub const SCOREBOARD_CAPTION: i32 = -1;

/// One ordered `C4Scoreboard::DoDlgShow` presentation reconciliation. The
/// dimensions and refcount are captured at call time because later SetCell
/// calls must not retroactively make an earlier empty-board request visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreboardPresentationRequest {
    pub rows: usize,
    pub columns: usize,
    pub show_count: i32,
}

impl ScoreboardPresentationRequest {
    pub fn should_be_shown(self) -> bool {
        self.show_count > 0 && self.rows != 0 && self.columns != 0
    }
}

#[derive(Debug, Default)]
pub(crate) struct ScoreboardPresentationSink {
    active: bool,
    pending: Vec<ScoreboardPresentationRequest>,
}

impl ScoreboardPresentationSink {
    pub(crate) fn begin_runtime_capture(&mut self) {
        // Initialize/save-load script activity occurs while C4GUI is still
        // exclusive and must never be replayed after entering the game.
        self.pending.clear();
        self.active = true;
    }

    pub(crate) fn apply_show_change(&mut self, scoreboard: &mut ScoreboardState, change: i32) {
        // C4Scoreboard::DoDlgShow returns before even changing iDlgShow while
        // the GUI is invalid/exclusive (C4Scoreboard.cpp:234-239).
        if !self.active {
            return;
        }
        scoreboard.adjust_show_count(change);
        self.pending.push(ScoreboardPresentationRequest {
            rows: scoreboard.row_count(),
            columns: scoreboard.column_count(),
            show_count: scoreboard.show_count(),
        });
    }

    pub(crate) fn drain(&mut self) -> Vec<ScoreboardPresentationRequest> {
        self.pending.drain(..).collect()
    }
}

/// One cell in the rectangular C4Scoreboard matrix. Row and column zero are
/// header cells; their `value` stores the corresponding lookup key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreboardCell {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default)]
    value: i32,
}

impl ScoreboardCell {
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn value(&self) -> i32 {
        self.value
    }
}

/// Script-controlled scoreboard state. The nested vectors retain C++'s
/// insertion order and rectangular row-major layout (`C4Scoreboard.h:40-42`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreboardState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rows: Vec<Vec<ScoreboardCell>>,
    #[serde(default, skip_serializing_if = "is_zero")]
    show_count: i32,
}

impl ScoreboardState {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    pub fn cell(&self, row: usize, column: usize) -> Option<&ScoreboardCell> {
        self.rows.get(row).and_then(|row| row.get(column))
    }

    pub fn show_count(&self) -> i32 {
        self.show_count
    }

    /// Whether the user may open this scoreboard with the global Tab binding
    /// (`C4Scoreboard::CanBeShown`, C4Scoreboard.h:83). Zero is deliberately
    /// eligible for a user toggle; a negative script refcount disables it.
    pub fn can_be_shown(&self) -> bool {
        self.show_count >= 0 && self.row_count() != 0 && self.column_count() != 0
    }

    /// Script-requested visibility (`C4Scoreboard::ShouldBeShown`,
    /// C4Scoreboard.h:75). The GUI may still suppress the dialog while an
    /// exclusive or game-over dialog is active.
    pub fn should_be_shown(&self) -> bool {
        self.show_count > 0 && self.row_count() != 0 && self.column_count() != 0
    }

    pub(crate) fn cell_by_key(&self, column_key: i32, row_key: i32) -> Option<&ScoreboardCell> {
        let column = self
            .rows
            .first()?
            .iter()
            .position(|cell| cell.value == column_key)?;
        let row = self
            .rows
            .iter()
            .position(|cells| cells[0].value == row_key)?;
        self.cell(row, column)
    }

    pub(crate) fn is_default(&self) -> bool {
        self.rows.is_empty() && self.show_count == 0
    }

    pub(crate) fn set_cell(
        &mut self,
        column_key: i32,
        row_key: i32,
        text: Option<String>,
        value: i32,
    ) {
        // SetCell first materializes the shared title corner
        // (C4Scoreboard.cpp:141-147).
        if self.rows.is_empty() {
            self.rows = vec![vec![ScoreboardCell {
                text: None,
                value: SCOREBOARD_CAPTION,
            }]];
        }

        let column = self
            .rows
            .first()
            .and_then(|header| header.iter().position(|cell| cell.value == column_key))
            .unwrap_or_else(|| {
                let column = self.column_count();
                self.rows
                    .iter_mut()
                    .for_each(|row| row.push(ScoreboardCell::default()));
                self.rows[0][column].value = column_key;
                column
            });
        let row = self
            .rows
            .iter()
            .position(|cells| cells[0].value == row_key)
            .unwrap_or_else(|| {
                let row = self.rows.len();
                let columns = self.column_count();
                self.rows.push(vec![ScoreboardCell::default(); columns]);
                self.rows[row][0].value = row_key;
                row
            });

        let prune = text.as_deref().is_none_or(str::is_empty);
        self.rows[row][column].text = text;
        if row != 0 && column != 0 {
            self.rows[row][column].value = value;
        }

        if prune {
            // The scan tests StdStrBuf's pointer truthiness, so an allocated
            // empty string still keeps its row/column alive
            // (C4Scoreboard.cpp:161-172; StdBuf.h:527).
            if row != 0 && self.rows[row][1..].iter().all(|cell| cell.text.is_none()) {
                self.rows.remove(row);
            }
            if column != 0
                && self
                    .rows
                    .iter()
                    .skip(1)
                    .all(|row| row[column].text.is_none())
            {
                self.rows.iter_mut().for_each(|row| {
                    row.remove(column);
                });
            }
        }
    }

    pub(crate) fn adjust_show_count(&mut self, change: i32) {
        self.show_count = self.show_count.wrapping_add(change);
    }

    pub(crate) fn sort_by(&mut self, column_key: i32, reverse: bool) -> bool {
        let Some(column) = self
            .rows
            .first()
            .and_then(|header| header.iter().position(|cell| cell.value == column_key))
        else {
            return false;
        };
        self.rows[1..].sort_by(|left, right| {
            let ordering = left[column].value.cmp(&right[column].value);
            if reverse {
                ordering.reverse()
            } else {
                ordering
            }
        });
        true
    }
}

const fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::{ScoreboardState, SCOREBOARD_CAPTION};

    #[test]
    fn user_and_script_visibility_use_distinct_refcount_thresholds() {
        let mut scoreboard = ScoreboardState::default();
        assert!(!scoreboard.can_be_shown());
        assert!(!scoreboard.should_be_shown());

        scoreboard.adjust_show_count(1);
        assert_eq!(scoreboard.show_count(), 1);
        assert!(!scoreboard.can_be_shown(), "an empty matrix cannot open");
        assert!(!scoreboard.should_be_shown());

        scoreboard.set_cell(
            SCOREBOARD_CAPTION,
            SCOREBOARD_CAPTION,
            Some("Scores".to_string()),
            0,
        );
        assert_eq!((scoreboard.row_count(), scoreboard.column_count()), (1, 1));
        assert!(scoreboard.can_be_shown());
        assert!(scoreboard.should_be_shown());

        scoreboard.adjust_show_count(-1);
        assert!(scoreboard.can_be_shown(), "zero permits the user toggle");
        assert!(!scoreboard.should_be_shown());

        scoreboard.adjust_show_count(-1);
        assert!(
            !scoreboard.can_be_shown(),
            "negative disables the user toggle"
        );
        assert!(!scoreboard.should_be_shown());
    }
}
