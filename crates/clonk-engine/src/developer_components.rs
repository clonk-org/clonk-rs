//! Editable scenario component hosts (Script / Title / Info).
//!
//! `C4ComponentHost` keeps a component's bytes, its filename, and a `Modified`
//! flag. The console's editors replace the bytes **only on OK**
//! (`C4ComponentHost.cpp:330-334`: `Data.Copy(text)` then `Modified = true`);
//! Cancel leaves the host untouched.
//!
//! Saving is where the flag matters (`C4ComponentHost.cpp:231-236`):
//!
//! ```cpp
//! if (!Modified) return true;                       // untouched: write nothing
//! if (!Data)     return hGroup.Delete(Filename);    // emptied: remove the file
//! return hGroup.Add(Filename, Data);
//! ```
//!
//! So clearing an editor **deletes** the component rather than writing an empty
//! file — and an unmodified host is never rewritten, which is what keeps a save
//! from touching components the user never opened.

/// What a save should do with one component host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentSaveAction {
    /// `!Modified` — the host is skipped entirely.
    Skip,
    /// `!Data` — the component is removed from the group.
    Delete { filename: String },
    /// The component is written with exactly these bytes.
    Write { filename: String, data: Vec<u8> },
}

/// One editable component (`C4ComponentHost`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentHost {
    filename: String,
    data: Vec<u8>,
    modified: bool,
}

impl ComponentHost {
    /// Loads a host from the group's current bytes. A freshly loaded host is
    /// unmodified, so a save leaves it alone.
    pub fn loaded(filename: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            filename: filename.into(),
            data,
            modified: false,
        }
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// The bytes an editor should open with.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn modified(&self) -> bool {
        self.modified
    }

    /// The editor was accepted: replace the bytes and mark the host modified
    /// (`C4ComponentHost.cpp:330-334`). Accepting identical text still marks it
    /// modified, exactly as C++ does — it does not compare.
    pub fn accept(&mut self, data: Vec<u8>) {
        self.data = data;
        self.modified = true;
    }

    /// The editor was cancelled: nothing changes, not even the flag.
    pub fn cancel(&mut self) {}

    /// `C4ComponentHost::Save` (`:231-236`).
    pub fn save_action(&self) -> ComponentSaveAction {
        if !self.modified {
            return ComponentSaveAction::Skip;
        }
        if self.data.is_empty() {
            return ComponentSaveAction::Delete {
                filename: self.filename.clone(),
            };
        }
        ComponentSaveAction::Write {
            filename: self.filename.clone(),
            data: self.data.clone(),
        }
    }

    /// Clears the modified flag after a successful save, so a second save is a
    /// no-op.
    pub fn mark_saved(&mut self) {
        self.modified = false;
    }
}

/// Whether the console may open a component editor at all.
///
/// `C4Console.cpp:1328-1351` refuses them outright in a network game.
pub fn component_editor_available(network_game: bool) -> bool {
    !network_game
}

/// Which component the console's Components menu edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditableComponent {
    Script,
    Title,
    Info,
}

impl EditableComponent {
    /// Whether accepting this editor must relink the whole script tree —
    /// `Game.ScriptEngine.ReLink(&Game.Defs)` runs only after the Script editor
    /// closes (`C4Console.cpp:1328-1351`).
    pub fn relinks_scripts(self) -> bool {
        matches!(self, Self::Script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Console.cpp:1328-1351; C4ComponentHost.cpp:231-236,330-334 — OK commits
    // exact bytes, Cancel does not mutate, an unmodified host is skipped, an
    // emptied one is deleted, and only Script relinks.
    #[test]
    fn console_component_editors_commit_bytes_and_relink_script() {
        // Network games refuse the editors outright.
        assert!(!component_editor_available(true));
        assert!(component_editor_available(false));

        let mut host = ComponentHost::loaded("Script.c", b"func Init() {}".to_vec());
        assert!(!host.modified());
        // A host nobody opened is never rewritten by a save.
        assert_eq!(host.save_action(), ComponentSaveAction::Skip);

        // Cancel leaves everything alone, including the flag.
        host.cancel();
        assert!(!host.modified());
        assert_eq!(host.data(), b"func Init() {}");
        assert_eq!(host.save_action(), ComponentSaveAction::Skip);

        // OK commits the exact bytes and marks the host modified.
        host.accept(b"func Init() { Log(\"hi\"); }".to_vec());
        assert!(host.modified());
        assert_eq!(host.data(), b"func Init() { Log(\"hi\"); }");
        assert_eq!(
            host.save_action(),
            ComponentSaveAction::Write {
                filename: "Script.c".to_owned(),
                data: b"func Init() { Log(\"hi\"); }".to_vec(),
            }
        );

        // A successful save clears the flag, so saving twice writes once.
        host.mark_saved();
        assert_eq!(host.save_action(), ComponentSaveAction::Skip);

        // Emptying the editor deletes the component rather than writing an
        // empty file (`if (!Data) return hGroup.Delete(Filename)`).
        host.accept(Vec::new());
        assert_eq!(
            host.save_action(),
            ComponentSaveAction::Delete {
                filename: "Script.c".to_owned()
            }
        );

        // Accepting identical text still marks modified — C++ does not compare.
        let mut unchanged = ComponentHost::loaded("Title.txt", b"Castle".to_vec());
        unchanged.accept(b"Castle".to_vec());
        assert!(unchanged.modified());
        assert_eq!(
            unchanged.save_action(),
            ComponentSaveAction::Write {
                filename: "Title.txt".to_owned(),
                data: b"Castle".to_vec(),
            }
        );

        // Only the Script editor relinks the script tree.
        assert!(EditableComponent::Script.relinks_scripts());
        assert!(!EditableComponent::Title.relinks_scripts());
        assert!(!EditableComponent::Info.relinks_scripts());
    }

    // C4Console.cpp:1335-1342 — accepting Script replaces the scenario body and
    // relinks the whole tree, without re-running Initialize.
    #[test]
    fn console_component_editors_commit_bytes_and_relink_script_through_the_engine() {
        use crate::{
            Engine, LegacyCString, ScriptControlData, ScriptControlPolicy, ScriptStrictness,
            SCRIPT_SCOPE_CONSOLE,
        };
        use clonk_script::Value;

        let call = |source: &str| ScriptControlData {
            target_object: SCRIPT_SCOPE_CONSOLE,
            strictness: ScriptStrictness::Strict3,
            script: LegacyCString::from_bytes(source.as_bytes().to_vec())
                .expect("fixture script contains no NUL"),
            by_client: 0,
        };

        let mut engine = Engine::with_seed(7);
        engine
            .install_scenario_script("Scenario", "#strict 3\nfunc Answer() { return 41; }")
            .expect("scenario script installs");
        assert_eq!(
            engine
                .execute_script_control(&call("Answer()"), ScriptControlPolicy::live(false))
                .expect("console scope executes"),
            Some(Value::Int(41))
        );

        // Accepting the editor swaps the body in place; the console sees the
        // new definition immediately.
        engine
            .apply_scenario_script_edit("Scenario", "#strict 3\nfunc Answer() { return 42; }")
            .expect("edited scenario script installs and relinks");
        assert_eq!(
            engine
                .execute_script_control(&call("Answer()"), ScriptControlPolicy::live(false))
                .expect("console scope executes"),
            Some(Value::Int(42)),
            "the edited body replaces the running scenario script"
        );

        // A function the edit dropped is gone rather than lingering from the
        // previous link.
        engine
            .apply_scenario_script_edit("Scenario", "#strict 3\nfunc Other() { return 1; }")
            .expect("second edit installs");
        assert_eq!(
            engine
                .execute_script_control(&call("Answer()"), ScriptControlPolicy::live(false))
                .expect("a removed function is fail-safe, not an error"),
            Some(Value::Nil)
        );

        // Cancelling still relinks — `ReLink` sits outside the `#ifdef _WIN32`
        // in C++, so it runs even when the dialog changed nothing.
        engine
            .relink_after_component_edit()
            .expect("an unchanged editor still relinks");
        assert_eq!(
            engine
                .execute_script_control(&call("Other()"), ScriptControlPolicy::live(false))
                .expect("console scope executes"),
            Some(Value::Int(1)),
            "a bare relink preserves the installed body"
        );
    }
}
