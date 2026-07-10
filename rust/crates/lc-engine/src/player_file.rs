//! Legacy `.c4p` player files: C4PlayerInfoCore (`Player.txt`) plus the
//! crew roster of `*.c4i` child groups (C4ObjectInfoList::Load,
//! C4ObjectInfoList.cpp:56-83). The join pipeline consumes this to mirror
//! `C4Player::Load` (C4Player.cpp:1089-1107).

use lc_resources::Group;

use crate::scenario::ScenarioError;

/// One crew-roster entry: C4ObjectInfoCore (C4InfoCore.cpp:526-548) with
/// the runtime recruitment flags (C4ObjectInfo::InAction / HasDied) that
/// `GetIdle` filters on (C4ObjectInfoList.cpp:113-142) — both start clear
/// when loaded from file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrewInfo {
    /// `id` (C4ID of the crew definition; "Clonk" defaults to none here —
    /// C4ID_None loads stay unresolvable like C++).
    pub id: String,
    /// `Name` (default "Clonk").
    pub name: String,
    /// `Rank` (default 0).
    pub rank: i32,
    /// `Experience` (default 0) — GetIdle prefers the highest.
    pub experience: i32,
    /// `Participation` (default 1) — GetIdle requires 1.
    pub participation: i32,
    /// Recruited this round (C4ObjectInfo::InAction).
    pub in_action: bool,
    /// Died this round (C4ObjectInfo::HasDied).
    pub has_died: bool,
}

impl CrewInfo {
    fn from_sections(sections: &[(String, Vec<(String, String)>)]) -> Self {
        let entry = |section: &str, key: &str| -> Option<String> {
            sections
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(section))
                .and_then(|(_, entries)| {
                    entries
                        .iter()
                        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
                        .map(|(_, value)| value.clone())
                })
        };
        let int = |section: &str, key: &str, default: i32| -> i32 {
            entry(section, key)
                .and_then(|value| parse_leading_i32(&value))
                .unwrap_or(default)
        };
        Self {
            id: entry("ObjectInfo", "id").unwrap_or_default(),
            name: entry("ObjectInfo", "Name").unwrap_or_else(|| "Clonk".to_string()),
            rank: int("ObjectInfo", "Rank", 0),
            experience: int("ObjectInfo", "Experience", 0),
            participation: int("ObjectInfo", "Participation", 1),
            in_action: false,
            has_died: false,
        }
    }
}

/// The parsed player file: C4PlayerInfoCore (C4InfoCore.cpp:148-177) and
/// the crew roster in group order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerFile {
    /// `[Player] Name` (default "Neuling").
    pub name: String,
    /// `[Player] Score`, the persistent settlement score
    /// (C4InfoCore.cpp:156; default 0).
    pub score: i32,
    /// `[Player] TotalPlayingTime` in seconds
    /// (C4InfoCore.cpp:160; default 0).
    pub total_playing_time: i32,
    /// `[Preferences] Color` — the indexed preferred color (default 0).
    pub pref_color: i32,
    /// `[Preferences] ColorDw` — 24-bit RGB preference (default 0xff).
    pub pref_color_dw: u32,
    /// `[Preferences] Position` — preferred start position (default 0).
    pub pref_position: i32,
    /// `[Preferences] AutoStopControl` — PrefControlStyle: Jump'n'Run
    /// control when 1 (C4InfoCore.cpp:170; default 0 = classic, :84).
    pub pref_control_style: bool,
    /// Crew roster, `*.c4i` entries in group order then subfolder recursion
    /// (C4ObjectInfoList.cpp:56-83).
    pub crew: Vec<CrewInfo>,
}

impl PlayerFile {
    pub fn load(group: &Group) -> Result<Self, ScenarioError> {
        let core_bytes = group.read_file("Player.txt")?;
        // Legacy files are ISO-8859-1/Windows-1252; lossy decode like the
        // other legacy readers.
        let core_text = String::from_utf8_lossy(&core_bytes);
        let sections = parse_ini_sections(&core_text);
        let entry = |section: &str, key: &str| -> Option<String> {
            sections
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(section))
                .and_then(|(_, entries)| {
                    entries
                        .iter()
                        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
                        .map(|(_, value)| value.clone())
                })
        };
        let int = |section: &str, key: &str, default: i32| -> i32 {
            entry(section, key)
                .and_then(|value| parse_leading_i32(&value))
                .unwrap_or(default)
        };

        let mut crew = Vec::new();
        collect_crew(group, &mut crew)?;

        Ok(Self {
            name: entry("Player", "Name").unwrap_or_else(|| "Neuling".to_string()),
            score: int("Player", "Score", 0),
            total_playing_time: int("Player", "TotalPlayingTime", 0),
            pref_color: int("Preferences", "Color", 0),
            pref_color_dw: entry("Preferences", "ColorDw")
                .and_then(|value| parse_leading_i32(&value))
                .map(|value| value as u32)
                .unwrap_or(0xff),
            pref_position: int("Preferences", "Position", 0),
            pref_control_style: int("Preferences", "AutoStopControl", 0) != 0,
            crew,
        })
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load(&group)
    }
}

/// `C4ObjectInfoList::Load` (C4ObjectInfoList.cpp:56-83): all `*.c4i`
/// child groups in entry order, then recursion into remaining subgroups.
fn collect_crew(group: &Group, crew: &mut Vec<CrewInfo>) -> Result<(), ScenarioError> {
    let mut subgroups = Vec::new();
    for entry in group.entries()? {
        if std::env::var("LC_C4P_DEBUG").is_ok() {
            eprintln!("C4P entry: {entry:?}");
        }
        let name = entry.relative_path.to_string_lossy().to_string();
        let is_info = name.to_ascii_lowercase().ends_with(".c4i");
        let Ok(child) = group.open_child(&entry.relative_path) else {
            continue;
        };
        if is_info {
            if let Ok(bytes) = child.read_file("ObjectInfo.txt") {
                let text = String::from_utf8_lossy(&bytes);
                let sections = parse_ini_sections(&text);
                crew.push(CrewInfo::from_sections(&sections));
            }
        } else if entry.is_directory {
            subgroups.push(child);
        }
    }
    for child in subgroups {
        collect_crew(&child, crew)?;
    }
    Ok(())
}

/// Minimal legacy INI reader: ordered sections of ordered key/value pairs,
/// `;`/`#`/`//` comments stripped (StdCompilerINIRead tolerances).
fn parse_ini_sections(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for raw_line in text.lines() {
        let mut line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find("//") {
            line = line[..idx].trim_end();
            if line.is_empty() {
                continue;
            }
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim().to_string();
            sections.push((name, Vec::new()));
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some((_, entries)) = sections.last_mut() {
            entries.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    sections
}

/// StdCompilerINIRead numbers parse strtol-style: leading integer, trailing
/// junk ignored.
fn parse_leading_i32(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    let end = trimmed
        .char_indices()
        .take_while(|&(index, ch)| ch.is_ascii_digit() || (index == 0 && (ch == '-' || ch == '+')))
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    trimmed[..end].parse::<i64>().ok().map(|v| v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_player_core_and_crew_roster_like_cpp() {
        // C4Player::Load (C4Player.cpp:1089-1107): C4PlayerInfoCore from
        // Player.txt (C4InfoCore.cpp:148-177) and the crew info list from
        // the *.c4i child groups (C4ObjectInfoList.cpp:56-83), each
        // carrying a C4ObjectInfoCore (C4InfoCore.cpp:526-548).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Tester.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\nRank=3\nScore=250\nTotalPlayingTime=1234\n\n[Preferences]\nColor=4\nColorDw=12345678\nPosition=2\nAutoStopControl=1\n",
        )
        .expect("write core");

        let first = root.join("Wipf.c4i");
        std::fs::create_dir_all(&first).expect("info dir");
        std::fs::write(
            first.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=COWB\nName=Wipf\nRank=2\nExperience=900\nParticipation=1\n\n[Physical]\nWalk=80000\n",
        )
        .expect("write info");

        let second = root.join("Zorro.c4i");
        std::fs::create_dir_all(&second).expect("info dir");
        std::fs::write(
            second.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=TRPR\nName=Zorro\nExperience=50\n",
        )
        .expect("write info");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        assert_eq!(player.name, "Tyler");
        // C4PlayerInfoCore::CompileFunc stores both values in [Player]
        // (C4InfoCore.cpp:148-161).
        assert_eq!(player.score, 250);
        assert_eq!(player.total_playing_time, 1_234);
        assert_eq!(player.pref_color, 4);
        assert_eq!(player.pref_color_dw, 12345678);
        assert_eq!(player.pref_position, 2);
        assert!(
            player.pref_control_style,
            "AutoStopControl=1 selects Jump'n'Run control (C4InfoCore.cpp:170)"
        );

        assert_eq!(player.crew.len(), 2);
        let wipf = player
            .crew
            .iter()
            .find(|info| info.name == "Wipf")
            .expect("Wipf parsed");
        assert_eq!(wipf.id, "COWB");
        assert_eq!(wipf.rank, 2);
        assert_eq!(wipf.experience, 900);
        assert_eq!(wipf.participation, 1);
        assert!(!wipf.in_action);
        assert!(!wipf.has_died);
        let zorro = player
            .crew
            .iter()
            .find(|info| info.name == "Zorro")
            .expect("Zorro parsed");
        assert_eq!(zorro.id, "TRPR");
        assert_eq!(zorro.rank, 0, "Rank defaults to 0");
        assert_eq!(zorro.participation, 1, "Participation defaults to 1");
    }

    #[test]
    fn missing_core_keys_fall_back_to_cpp_defaults() {
        // C4PlayerInfoCore defaults (C4InfoCore.cpp:152,166-173):
        // Name "Neuling", Color 0, ColorDw 0xff, Position 0.
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Empty.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(root.join("Player.txt"), "[Player]\n").expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        assert_eq!(player.name, "Neuling");
        assert_eq!(player.score, 0);
        assert_eq!(player.total_playing_time, 0);
        assert_eq!(player.pref_color, 0);
        assert_eq!(player.pref_color_dw, 0xff);
        assert_eq!(player.pref_position, 0);
        assert!(
            !player.pref_control_style,
            "AutoStopControl defaults to 0 = classic (C4InfoCore.cpp:84)"
        );
        assert!(player.crew.is_empty());
    }
}
