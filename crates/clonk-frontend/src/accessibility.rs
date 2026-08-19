//! Platform-independent accessibility semantics for the raster frontend.
//!
//! The frontend draws its own widgets, so a control's role, name and value
//! exist only as pixels: a screen reader sees an opaque window
//! (clonk-org/clonk-rs#392). Nothing here talks to a platform API — this is the
//! *content* half, the description a bridge publishes, kept separate so it can
//! be tested on a machine with no assistive technology installed at all.
//!
//! Only the scenario-selection screen is modelled, because that is where
//! clonk-org/clonk-rs#392 names the gap: the search field has no role or name,
//! and the result count and no-result guidance are drawn but never announced.

/// What a control is, in the vocabulary every platform accessibility API
/// shares. Deliberately small: a role is only worth adding once something
/// publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// A single-line editable text field.
    TextInput,
    /// A region whose text changes are announced without moving focus. The
    /// search result count has to be one of these: it changes as a
    /// consequence of typing, and a reader that only spoke it on focus would
    /// never say it.
    Status,
}

/// One accessible control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub role: Role,
    /// The programmatic name — what the control *is*, not what it holds.
    pub name: String,
    /// The current content, for a role that has one.
    pub value: Option<String>,
    pub focused: bool,
}

/// The accessible description of the scenario-selection screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenSelSemantics {
    pub nodes: Vec<Node>,
}

impl ScenSelSemantics {
    /// The first node with `role`, for a bridge that publishes one of each.
    pub fn node(&self, role: Role) -> Option<&Node> {
        self.nodes.iter().find(|node| node.role == role)
    }
}

/// Describe the scenario-selection screen for a platform bridge.
///
/// `caption` and `empty_message` are the strings the screen already draws —
/// the count ("3 of 40 scenarios") and the guidance shown when nothing
/// matches. They are joined into one status because a reader announces a live
/// region as a unit, and hearing the count without the guidance is the case
/// clonk-org/clonk-rs#392 calls out.
pub fn scen_sel_semantics(
    search_text: &str,
    search_focused: bool,
    caption: Option<&str>,
    empty_message: Option<&str>,
) -> ScenSelSemantics {
    let mut nodes = vec![Node {
        role: Role::TextInput,
        name: "Scenario search".to_string(),
        value: Some(search_text.to_string()),
        focused: search_focused,
    }];

    let status = [caption, empty_message]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !status.is_empty() {
        nodes.push(Node {
            role: Role::Status,
            name: "Search results".to_string(),
            value: Some(status),
            focused: false,
        });
    }

    ScenSelSemantics { nodes }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// clonk-org/clonk-rs#392: "the search field has no semantic role or
    /// programmatic name".
    #[test]
    fn the_search_field_carries_a_role_a_name_and_its_current_text() {
        let semantics = scen_sel_semantics("cast", true, None, None);
        let field = semantics.node(Role::TextInput).expect("a search field");
        assert_eq!(field.name, "Scenario search");
        assert_eq!(field.value.as_deref(), Some("cast"));
        assert!(field.focused);
    }

    /// The field is described whether or not it is focused: a reader walks the
    /// window before anything has focus, and a field that only appears once
    /// focused cannot be found that way.
    #[test]
    fn the_search_field_is_described_even_when_it_does_not_have_focus() {
        let semantics = scen_sel_semantics("", false, None, None);
        let field = semantics.node(Role::TextInput).expect("a search field");
        assert!(!field.focused);
        assert_eq!(field.value.as_deref(), Some(""));
    }

    /// clonk-org/clonk-rs#392: "scenario-search counts and no-result guidance
    /// are visible pixels but are not announced through an accessibility
    /// status node".
    #[test]
    fn the_result_count_is_a_status_node_so_a_reader_announces_it_while_typing() {
        let semantics = scen_sel_semantics("cast", true, Some("3 of 40 scenarios"), None);
        let status = semantics.node(Role::Status).expect("a status node");
        assert_eq!(status.value.as_deref(), Some("3 of 40 scenarios"));
        assert!(
            !status.focused,
            "a status is announced without taking focus, or typing would lose the caret"
        );
    }

    /// A search that matches nothing has to announce *both* halves: the count
    /// alone says "No matches among 40 scenarios" without saying what was
    /// searched for, which is the guidance the screen draws underneath it.
    #[test]
    fn a_search_with_no_matches_announces_the_count_and_the_guidance_together() {
        let semantics = scen_sel_semantics(
            "zzz",
            true,
            Some("No matches among 40 scenarios"),
            Some("No scenarios match \"zzz\"."),
        );
        assert_eq!(
            semantics
                .node(Role::Status)
                .and_then(|node| node.value.as_deref()),
            Some("No matches among 40 scenarios No scenarios match \"zzz\".")
        );
    }

    /// With no search running there is nothing to announce, and an empty live
    /// region is worse than none: a reader may speak the silence as a change.
    #[test]
    fn an_idle_screen_publishes_no_status_node_at_all() {
        let semantics = scen_sel_semantics("", false, None, None);
        assert!(semantics.node(Role::Status).is_none());
        assert_eq!(semantics.nodes.len(), 1);
    }
}
