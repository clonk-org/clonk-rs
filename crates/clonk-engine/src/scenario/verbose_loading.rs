//! `Graphics.VerboseObjectLoading` definition diagnostics.
//!
//! `C4Config.cpp:453` defines the level with default 0. Three sites consume it:
//!
//! - `C4Def.cpp:555-556` logs each definition's group full name at level 3.
//! - `C4Def.cpp:1051-1060` logs `IDS_PRC_DEFOVERLOAD` at level 1 and the two
//!   old/overloading filename lines at level 2.
//! - `C4Particles.cpp:182-185` logs `IDS_PRC_DEFOVERLOAD` for particle
//!   overloads at level 1, with `"<particle>"` in place of the old id.
//!
//! Each level is a floor, not a selector: level 3 emits all three.

use std::sync::atomic::{AtomicI32, Ordering};

/// `Config.Graphics.VerboseObjectLoading`, default 0 (`C4Config.cpp:453`).
/// Process-global for the same reason C++'s `Config` is: the definition loader
/// reads it far from where the app parses the configuration.
static VERBOSE_OBJECT_LOADING: AtomicI32 = AtomicI32::new(0);

/// Publishes the configured level. The app calls this during startup; headless
/// engines keep the C++ default of 0.
pub fn set_verbose_object_loading(level: i32) {
    VERBOSE_OBJECT_LOADING.store(level, Ordering::Relaxed);
}

/// The active level.
pub fn verbose_object_loading() -> i32 {
    VERBOSE_OBJECT_LOADING.load(Ordering::Relaxed)
}

/// The shipped US `IDS_PRC_DEFOVERLOAD` text
/// (`planet/System.c4g/LanguageUS.txt:1197`). The app overwrites this from the
/// installed language table, matching the other process-global `LoadResStr`
/// entries in this crate.
pub const DEFAULT_DEFINITION_OVERLOAD_TEMPLATE: &str = "%s (%s) overloaded.";

/// `Log(IDS_PRC_DEFOVERLOAD, name, id)` — the two positional `%s` slots.
fn format_overload(template: &str, name: &str, id: &str) -> String {
    template.replacen("%s", name, 1).replacen("%s", id, 1)
}

/// A definition group's full name, which is what C++ stores in `C4Def::Filename`
/// and logs at levels 2 and 3. The port keeps `<group full name>/Script.c` as
/// `ScenarioDefinition::script_name`, so the script component is dropped.
pub fn group_full_name(script_name: &str) -> &str {
    script_name.strip_suffix("/Script.c").unwrap_or(script_name)
}

/// `C4Def.cpp:555-556` — the loaded definition's group full name, at level 3.
pub fn definition_loaded_line(level: i32, group_full_name: &str) -> Option<String> {
    (level >= 3).then(|| group_full_name.to_owned())
}

/// `C4Def.cpp:1051-1060` — the overload notice at level 1, followed by the old
/// and overloading group names at level 2. `overloading` is the definition that
/// wins; `old_*` describe the one it replaces.
pub fn definition_overload_lines(
    level: i32,
    template: &str,
    overloading_name: &str,
    old_id: &str,
    old_group_full_name: &str,
    overloading_group_full_name: &str,
) -> Vec<String> {
    if level < 1 {
        return Vec::new();
    }
    let mut lines = vec![format_overload(template, overloading_name, old_id)];
    if level >= 2 {
        // The leading spaces are the C++ format strings' alignment (:1057-1058).
        lines.push(format!("      Old def at {old_group_full_name}"));
        lines.push(format!("     Overload by {overloading_group_full_name}"));
    }
    lines
}

/// `C4Particles.cpp:182-185` — the overloaded particle's name against the
/// literal `"<particle>"`, at level 1.
pub fn particle_overload_line(level: i32, template: &str, old_name: &str) -> Option<String> {
    (level >= 1).then(|| format_overload(template, old_name, "<particle>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Def.cpp:555-556,1051-1060; C4Particles.cpp:182-185 — every level is a
    // floor, so nothing below its threshold is emitted and level 3 emits all.
    #[test]
    fn verbose_object_loading_levels_gate_definition_diagnostics() {
        const TEMPLATE: &str = DEFAULT_DEFINITION_OVERLOAD_TEMPLATE;
        let overload = |level| {
            definition_overload_lines(
                level,
                TEMPLATE,
                "Rock",
                "ROCK",
                "Objects.c4d/Rock.c4d",
                "Mods.c4d/Rock.c4d",
            )
        };

        // Level 0 is the default and emits nothing at any site (C4Config.cpp:453).
        assert_eq!(overload(0), Vec::<String>::new());
        assert_eq!(particle_overload_line(0, TEMPLATE, "Fire"), None);
        assert_eq!(definition_loaded_line(0, "Objects.c4d/Rock.c4d"), None);

        // Level 1 adds the overload notices only (:1051; C4Particles.cpp:182).
        assert_eq!(overload(1), vec!["Rock (ROCK) overloaded.".to_owned()]);
        assert_eq!(
            particle_overload_line(1, TEMPLATE, "Fire").as_deref(),
            Some("Fire (<particle>) overloaded.")
        );
        assert_eq!(definition_loaded_line(1, "Objects.c4d/Rock.c4d"), None);

        // Level 2 adds the old/overloading detail lines (:1055-1058).
        assert_eq!(
            overload(2),
            vec![
                "Rock (ROCK) overloaded.".to_owned(),
                "      Old def at Objects.c4d/Rock.c4d".to_owned(),
                "     Overload by Mods.c4d/Rock.c4d".to_owned(),
            ]
        );
        assert_eq!(definition_loaded_line(2, "Objects.c4d/Rock.c4d"), None);

        // Level 3 adds the loaded-filename line and keeps the rest (:555-556).
        assert_eq!(overload(3).len(), 3);
        assert_eq!(
            definition_loaded_line(3, "Objects.c4d/Rock.c4d").as_deref(),
            Some("Objects.c4d/Rock.c4d")
        );
    }

    // The port stores `<group full name>/Script.c`; C++ logs the group name.
    #[test]
    fn group_full_name_drops_the_script_component() {
        assert_eq!(
            group_full_name("Objects.c4d/Rock.c4d/Script.c"),
            "Objects.c4d/Rock.c4d"
        );
        // A definition with no script keeps whatever name it has.
        assert_eq!(
            group_full_name("Objects.c4d/Rock.c4d"),
            "Objects.c4d/Rock.c4d"
        );
    }

    // The template carries two positional %s, like `Log`'s varargs (:1053).
    #[test]
    fn overload_template_fills_both_positional_slots() {
        assert_eq!(
            format_overload("%s (%s) overloaded.", "Rock", "ROCK"),
            "Rock (ROCK) overloaded."
        );
        // A localized template with different surrounding text still fills in order.
        assert_eq!(
            format_overload("%s (%s) ueberladen.", "Stein", "ROCK"),
            "Stein (ROCK) ueberladen."
        );
    }
}
