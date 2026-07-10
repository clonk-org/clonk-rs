use serde::{Deserialize, Serialize};

/// The key shared by the caption row and caption column
/// (`C4Scoreboard::TitleKey`, C4Scoreboard.h:29).
pub const SCOREBOARD_CAPTION: i32 = -1;

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

    pub(crate) fn is_default(&self) -> bool {
        self.rows.is_empty() && self.show_count == 0
    }
}

const fn is_zero(value: &i32) -> bool {
    *value == 0
}
