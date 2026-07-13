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
    /// `[Preferences] Control` — raw preferred control set. Synthetic cores
    /// start on Keyboard1 (0); loaded files default an omitted key to
    /// Keyboard2 (1).
    pub pref_control: i32,
    /// `[Preferences] Mouse` — whether this player initially requests mouse
    /// control. C++ treats every nonzero stored integer as enabled.
    pub pref_mouse: bool,
    /// `[Preferences] AutoStopControl` — PrefControlStyle: Jump'n'Run
    /// control when 1 (C4InfoCore.cpp:170; default 0 = classic, :84).
    pub pref_control_style: bool,
    /// `[Preferences] AutoContextMenu` — automatically open context menus
    /// when entering opted-in containers. If omitted, C++ defaults this to
    /// `pref_control_style` (C4InfoCore.cpp:103-115,171).
    pub pref_auto_context_menu: bool,
    /// Crew roster, `*.c4i` entries in group order then subfolder recursion
    /// (C4ObjectInfoList.cpp:56-83).
    pub crew: Vec<CrewInfo>,
}

impl Default for PlayerFile {
    fn default() -> Self {
        Self {
            name: "Neuling".to_string(),
            score: 0,
            total_playing_time: 0,
            pref_color: 0,
            pref_color_dw: 0xff,
            pref_position: 0,
            pref_control: 0,
            pref_mouse: true,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
        }
    }
}

impl PlayerFile {
    /// `C4PlayerInfoCore::GetPrefColorValue`: use the 24-bit ColorDw when
    /// nonzero, otherwise map the indexed legacy color with the stock table.
    pub fn normalized_preferred_color(&self) -> u32 {
        if self.pref_color_dw != 0 {
            return self.pref_color_dw & 0x00ff_ffff;
        }
        const PLAYER_COLORS: [u32; 12] = [
            0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xc48444, 0x784830, 0xa04400, 0xf08050,
            0x848484, 0xffffff, 0x0094f8, 0xbc00c0,
        ];
        usize::try_from(self.pref_color)
            .ok()
            .and_then(|index| PLAYER_COLORS.get(index))
            .copied()
            .unwrap_or(0xaaaaaa)
    }

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
        let exact_int = |section: &str, key: &str, default: i32| -> i32 {
            sections
                .iter()
                .find(|(name, _)| name == section)
                .and_then(|(_, entries)| {
                    entries
                        .iter()
                        .find(|(entry_key, _)| entry_key == key)
                        .map(|(_, value)| value)
                })
                .and_then(|value| parse_leading_i32(value))
                .unwrap_or(default)
        };

        let mut crew = Vec::new();
        collect_crew(group, &mut crew)?;
        let pref_control_style = int("Preferences", "AutoStopControl", 0) != 0;
        let pref_auto_context_menu = match int("Preferences", "AutoContextMenu", -1) {
            -1 => pref_control_style,
            value => value != 0,
        };

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
            pref_control: exact_int("Preferences", "Control", 1),
            pref_mouse: exact_int("Preferences", "Mouse", 1) != 0,
            pref_control_style,
            pref_auto_context_menu,
            crew,
        })
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load(&group)
    }

    pub fn load_from_bytes(path: std::path::PathBuf, data: Vec<u8>) -> Result<Self, ScenarioError> {
        let group = Group::from_memory(path, data)?;
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
    let trimmed = value.trim_start();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        let end = hex
            .char_indices()
            .take_while(|&(_, ch)| ch.is_ascii_hexdigit())
            .map(|(index, ch)| index + ch.len_utf8())
            .last()?;
        return i64::from_str_radix(&hex[..end], 16)
            .ok()
            .map(|value| value as i32);
    }
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
    fn synthetic_player_core_defaults_to_keyboard_one_with_mouse() {
        // C4PlayerInfoCore::Default is used for synthetic/script cores and
        // freshly created players. It selects Keyboard1 and enables mouse
        // preference (pristine 9ffa0a5d src/C4InfoCore.cpp:66-85;
        // src/C4StartupPlrSelDlg.cpp:1103-1114).
        let player = PlayerFile::default();

        assert_eq!(player.pref_control, 0);
        assert!(player.pref_mouse);
    }

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
            "[Player]\nName=Tyler\nRank=3\nScore=250\nTotalPlayingTime=1234\n\n[Preferences]\nColor=4\nColorDw=12345678\nPosition=2\nControl=3\nMouse=0\nAutoStopControl=1\n",
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
        assert_eq!(player.pref_control, 3);
        assert!(!player.pref_mouse);
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
        assert_eq!(
            player.pref_control, 1,
            "omitted loaded-file Control defaults to Keyboard2"
        );
        assert!(player.pref_mouse, "omitted Mouse defaults to enabled");
        assert!(
            !player.pref_control_style,
            "AutoStopControl defaults to 0 = classic (C4InfoCore.cpp:84)"
        );
        assert!(
            !player.pref_auto_context_menu,
            "AutoContextMenu inherits the default classic style (C4InfoCore.cpp:103-115)"
        );
        assert!(player.crew.is_empty());
    }

    #[test]
    fn control_and_mouse_names_are_exact_case_like_cpp() {
        // StdCompilerINIRead compares section and value names exactly. The
        // lowercase variants are unexpected entries, so CompileFunc applies
        // its loaded-file defaults (pristine 9ffa0a5d
        // src/StdCompiler.cpp:498-525; src/C4InfoCore.cpp:164-174).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Case.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[preferences]\nControl=3\nMouse=0\n\n[Preferences]\ncontrol=2\nmouse=0\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert_eq!(player.pref_control, 1);
        assert!(player.pref_mouse);
    }

    #[test]
    fn control_and_mouse_accept_cpp_hex_numbers_with_trailing_text() {
        // StdCompilerINIRead selects base 16 for a 0x prefix and strtol stops
        // at the first non-digit (pristine 9ffa0a5d
        // src/StdCompiler.h:705-722; src/StdCompiler.cpp:646-649).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Hex.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Preferences]\nControl=0x4gamepad\nMouse=0x0mouse\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert_eq!(player.pref_control, 4);
        assert!(!player.pref_mouse);
    }

    #[test]
    fn loads_explicit_auto_context_menu_preference_like_cpp() {
        // C4PlayerInfoCore::CompileFunc reads [Preferences] AutoContextMenu
        // as PrefAutoContextMenu (src/C4InfoCore.cpp:164-172).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("AutoMenu.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nAutoContextMenu=1\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert!(player.pref_auto_context_menu);
    }

    #[test]
    fn omitted_auto_context_menu_defaults_to_control_style_like_cpp() {
        // C4PlayerInfoCore::CompileFunc defaults AutoContextMenu to -1;
        // C4PlayerInfoCore::Load then replaces -1 with PrefControlStyle
        // (src/C4InfoCore.cpp:103-115,164-172).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("DefaultAutoMenu.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nAutoStopControl=1\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert!(player.pref_auto_context_menu);
    }

    #[test]
    fn loads_cpp_packed_player_data_from_memory() {
        // Remote C4ControlJoinPlayer saves its PlrData blob as a temporary
        // .c4p and C4Player::Load opens that packed group
        // (src/C4Control.cpp:731-744; src/C4Player.cpp:267-284,1089-1106).
        let bytes = include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec();

        let player =
            PlayerFile::load_from_bytes(std::path::PathBuf::from("embedded_player.c4p"), bytes)
                .expect("C++-packed PlrData loads");

        assert_eq!(player.name, "Embedded Tyler");
        assert_eq!((player.score, player.total_playing_time), (42, 99));
        assert_eq!((player.pref_color, player.pref_position), (3, 2));
        assert_eq!(player.pref_color_dw, 1_122_867);
        assert!(player.pref_control_style);
        assert!(!player.pref_auto_context_menu);
    }
}
