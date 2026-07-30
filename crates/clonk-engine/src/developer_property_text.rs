//! The developer property panel's text composition.
//!
//! `C4PropertyDlg::Update` (`C4PropertyDlg.cpp:169-256`) switches on the
//! selection count: none yields `IDS_CNS_NOOBJECT`, many yields
//! `IDS_CNS_MULTIPLEOBJECTS` with the count, and exactly one composes the
//! object's detail in a fixed order — type, then owner, contents, action,
//! locals and effects, each section emitted only when it has something to say.
//!
//! The section *values* (locals, effects, contents names) are supplied by the
//! caller so this stays independent of the engine's value formatting, which is
//! tracked separately as M10-P4-L085.

/// The resolved resource strings the panel needs. Kept as owned text so the
/// caller resolves them through the active language table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyPanelStrings {
    /// `IDS_CNS_NOOBJECT`.
    pub no_object: String,
    /// `IDS_CNS_TYPE`, with `%s` name and `%s` id.
    pub type_line: String,
    /// `IDS_CNS_OWNER`, with `%s` player name.
    pub owner: String,
    /// `IDS_CNS_CONTENTS`.
    pub contents: String,
    /// `IDS_CNS_ACTION`.
    pub action: String,
    /// `IDS_CNS_LOCALS`.
    pub locals: String,
    /// `IDS_CNS_EFFECTS`.
    pub effects: String,
    /// `IDS_CNS_MULTIPLEOBJECTS`, with `%d` count.
    pub multiple_objects: String,
}

/// One selected object's already-formatted detail.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PropertyPanelObject {
    /// `cobj->GetName()`.
    pub name: String,
    /// `C4IdText(cobj->Def->id)`.
    pub id: String,
    /// The owning player's name, when `ValidPlr(cobj->Owner)` (`:190-194`).
    pub owner: Option<String>,
    /// `cobj->Contents.GetNameList(...)`, when the list is non-empty (`:196-201`).
    pub contents: Option<String>,
    /// The current action's name, when `Action.Act != ActIdle` (`:203-208`).
    pub action: Option<String>,
    /// Local entries in C++'s emission order — indexed first, then named
    /// (`:210-234`). Each is a fully formatted line body.
    pub locals: Vec<String>,
    /// Effect entries, `" {name}: Interval {interval}"` (`:236-247`).
    pub effects: Vec<String>,
}

/// Substitutes positional `%s`/`%d` in a resource template, in order.
fn substitute(template: &str, arguments: &[&str]) -> String {
    let mut result = template.to_owned();
    for argument in arguments {
        if let Some(index) = result.find("%s").or_else(|| result.find("%d")) {
            result.replace_range(index..index + 2, argument);
        }
    }
    result
}

/// `C4PropertyDlg::Update`'s body for a selection of `selection_count` objects.
///
/// `object` is consulted only when exactly one is selected — C++ reads
/// `Selection.GetObject()` in that arm alone.
pub fn property_panel_text(
    strings: &PropertyPanelStrings,
    selection_count: usize,
    object: Option<&PropertyPanelObject>,
) -> String {
    match (selection_count, object) {
        // "No selection" (:180-183).
        (0, _) => strings.no_object.clone(),
        (1, Some(object)) => single_object_text(strings, object),
        // A single selection with no detail still falls back to the empty text
        // rather than composing a blank record.
        (1, None) => strings.no_object.clone(),
        // "Multiple selected objects" (:252-255).
        (count, _) => substitute(&strings.multiple_objects, &[&count.to_string()]),
    }
}

fn single_object_text(strings: &PropertyPanelStrings, object: &PropertyPanelObject) -> String {
    // Type is unconditional and heads the record (:187-189).
    let mut text = substitute(&strings.type_line, &[&object.name, &object.id]);
    if let Some(owner) = &object.owner {
        text.push('\n');
        text.push_str(&substitute(&strings.owner, &[owner]));
    }
    if let Some(contents) = &object.contents {
        text.push('\n');
        text.push_str(&strings.contents);
        text.push_str(contents);
    }
    if let Some(action) = &object.action {
        text.push('\n');
        text.push_str(&strings.action);
        text.push_str(action);
    }
    // The locals header is emitted once, before the first entry, and covers
    // both the indexed and named lists (`fFirstLocal`, :212-234).
    if let Some((first, rest)) = object.locals.split_first() {
        text.push('\n');
        text.push_str(&strings.locals);
        text.push('\n');
        text.push_str(first);
        for local in rest {
            text.push('\n');
            text.push_str(local);
        }
    }
    // The effects header likewise appears only with the first effect (:238-247).
    if let Some((first, rest)) = object.effects.split_first() {
        text.push('\n');
        text.push_str(&strings.effects);
        text.push('\n');
        text.push_str(first);
        for effect in rest {
            text.push('\n');
            text.push_str(effect);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings() -> PropertyPanelStrings {
        PropertyPanelStrings {
            no_object: "No object selected.".into(),
            type_line: "Type: %s (%s)".into(),
            owner: "Owner: %s".into(),
            contents: "Contents: ".into(),
            action: "Action: ".into(),
            locals: "Local variables:".into(),
            effects: "Effects:".into(),
            multiple_objects: "%d objects selected.".into(),
        }
    }

    // C4PropertyDlg.cpp:169-256 — the 0/1/many switch and the fixed section
    // order, each section appearing only when it has content.
    #[test]
    fn object_list_and_property_dialog_share_edit_cursor_selection_order() {
        let strings = strings();

        // No selection, and a many-selection count line.
        assert_eq!(
            property_panel_text(&strings, 0, None),
            "No object selected."
        );
        assert_eq!(
            property_panel_text(&strings, 4, None),
            "4 objects selected."
        );

        // A bare object shows only its type line — every other section is
        // conditional.
        let bare = PropertyPanelObject {
            name: "Rock".into(),
            id: "ROCK".into(),
            ..PropertyPanelObject::default()
        };
        assert_eq!(
            property_panel_text(&strings, 1, Some(&bare)),
            "Type: Rock (ROCK)"
        );

        // Full record, in C++'s order: type, owner, contents, action, locals,
        // effects — with each header emitted once.
        let full = PropertyPanelObject {
            name: "Clonk".into(),
            id: "CLNK".into(),
            owner: Some("Red".into()),
            contents: Some("Rock, Wood".into()),
            action: Some("Walk".into()),
            locals: vec![" Local(0) = 5".into(), " speed = 12".into()],
            effects: vec![" Fire: Interval 1".into(), " Smoke: Interval 3".into()],
        };
        assert_eq!(
            property_panel_text(&strings, 1, Some(&full)),
            concat!(
                "Type: Clonk (CLNK)\n",
                "Owner: Red\n",
                "Contents: Rock, Wood\n",
                "Action: Walk\n",
                "Local variables:\n",
                " Local(0) = 5\n",
                " speed = 12\n",
                "Effects:\n",
                " Fire: Interval 1\n",
                " Smoke: Interval 3",
            )
        );

        // The locals header appears once even with a single entry, and not at
        // all with none.
        let one_local = PropertyPanelObject {
            locals: vec![" only = 1".into()],
            ..bare.clone()
        };
        assert_eq!(
            property_panel_text(&strings, 1, Some(&one_local)),
            "Type: Rock (ROCK)\nLocal variables:\n only = 1"
        );

        // Effects can appear without locals, and keep their own header.
        let effects_only = PropertyPanelObject {
            effects: vec![" Fire: Interval 1".into()],
            ..bare.clone()
        };
        assert_eq!(
            property_panel_text(&strings, 1, Some(&effects_only)),
            "Type: Rock (ROCK)\nEffects:\n Fire: Interval 1"
        );

        // One selected object with no detail falls back rather than composing
        // a blank record.
        assert_eq!(
            property_panel_text(&strings, 1, None),
            "No object selected."
        );
    }
}
