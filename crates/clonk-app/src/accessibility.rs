//! The platform half of the frontend's accessibility semantics.
//!
//! `clonk_frontend::accessibility` describes the scenario-selection screen in
//! toolkit-independent terms; nothing there talks to a platform API. This
//! module is the bridge that publishes that description through AccessKit, so
//! VoiceOver, NVDA and Orca see a search field and a live result status
//! instead of one opaque window (clonk-org/clonk-rs#392).
//!
//! It is presentation-only. Nothing here reads or writes simulation state,
//! control synchronization or the committed text of the search field.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Live,
    Node as PlatformNode, NodeId, Role as PlatformRole, Tree, TreeId, TreeUpdate,
};
use accesskit_winit::Adapter;
use clonk_frontend::accessibility::{Node, Role, ScenSelSemantics};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// The carrier window. AccessKit needs a root even when the screen underneath
/// it describes nothing, so this node is always published.
const WINDOW: NodeId = NodeId(0);

/// Node identities are derived from the role rather than from a running
/// counter: a reader tracks a control across updates by its id, and a field
/// that changed id on every keystroke would be announced as a new control.
///
/// `None` for a role this bridge cannot publish. `Role` is `#[non_exhaustive]`,
/// so the content model can grow a role first; leaving it out is better than
/// guessing an id, because two roles sharing one would merge two controls.
fn node_id(role: Role) -> Option<NodeId> {
    match role {
        Role::TextInput => Some(NodeId(1)),
        Role::Status => Some(NodeId(2)),
        _ => None,
    }
}

fn platform_role(role: Role) -> PlatformRole {
    match role {
        Role::TextInput => PlatformRole::TextInput,
        // AccessKit has no dedicated status role; a label whose liveness is
        // set is the shape every platform adapter maps to a live region.
        _ => PlatformRole::Label,
    }
}

fn platform_node(node: &Node) -> PlatformNode {
    let mut published = PlatformNode::new(platform_role(node.role));
    published.set_label(node.name.clone());
    if let Some(value) = node.value.as_ref() {
        published.set_value(value.clone());
    }
    if node.role == Role::Status {
        // Polite, not assertive: the count changes on every keystroke, and an
        // assertive region interrupts the letter the reader is echoing back.
        published.set_live(Live::Polite);
    }
    published
}

/// Describe the whole window for AccessKit.
///
/// The update is always a complete tree. The adapter is created with an
/// activation handler that returns no initial tree, and AccessKit requires a
/// full tree in that case; sending one every time also keeps this a pure
/// function of the semantics, with no incremental state to fall out of sync.
pub(crate) fn window_tree_update(semantics: &ScenSelSemantics, window_title: &str) -> TreeUpdate {
    let mut window = PlatformNode::new(PlatformRole::Window);
    window.set_label(window_title.to_string());
    let published = semantics
        .nodes
        .iter()
        .filter_map(|node| node_id(node.role).map(|id| (id, node)))
        .collect::<Vec<_>>();
    window.set_children(published.iter().map(|(id, _)| *id).collect::<Vec<_>>());

    let focus = published
        .iter()
        .find_map(|(id, node)| node.focused.then_some(*id))
        .unwrap_or(WINDOW);

    let mut nodes = vec![(WINDOW, window)];
    nodes.extend(
        published
            .into_iter()
            .map(|(id, node)| (id, platform_node(node))),
    );

    TreeUpdate {
        nodes,
        tree: Some(Tree::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// What the platform tree currently says, so an unchanged screen is not
/// described again.
///
/// The event loop offers the whole description once per turn; nearly every
/// turn repeats the previous one, and AccessKit walks every node of an update
/// whether or not it changed anything.
#[derive(Default)]
pub(crate) struct PublishedWindow {
    published: Option<(ScenSelSemantics, String)>,
}

impl PublishedWindow {
    /// The tree to hand AccessKit, or `None` when it already says this.
    pub(crate) fn update(
        &mut self,
        semantics: ScenSelSemantics,
        window_title: &str,
    ) -> Option<TreeUpdate> {
        let described = (semantics, window_title.to_string());
        (self.published.as_ref() != Some(&described)).then(|| {
            let update = window_tree_update(&described.0, &described.1);
            self.published = Some(described);
            update
        })
    }

    /// Forget what was published, so the next description is sent in full.
    ///
    /// A reader that attaches to an already-open window asks AccessKit for an
    /// initial tree; nothing on screen changed, so without this the next
    /// `update` would answer `None` and the reader would keep the placeholder.
    pub(crate) fn invalidate(&mut self) {
        self.published = None;
    }
}

/// Raised when AccessKit wants the tree from scratch.
///
/// AccessKit calls its handlers on whichever thread the platform adapter runs
/// on, so the request cannot be answered where it arrives; it is latched here
/// and drained on the next event-loop turn, which is the only place the screen
/// can be described.
#[derive(Clone, Default)]
struct TreeRequest(Arc<AtomicBool>);

impl TreeRequest {
    fn raise(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

impl ActivationHandler for TreeRequest {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.raise();
        // No tree synchronously: the description lives in the application
        // state this handler's thread must not touch. The platform adapter
        // shows a placeholder until the next event-loop turn answers.
        None
    }
}

impl DeactivationHandler for TreeRequest {
    fn deactivate_accessibility(&mut self) {
        // The next reader to attach raises `request_initial_tree` again, but
        // the published description is stale from this moment, and clearing
        // it here means the reattach cannot race a screen change that would
        // otherwise dedup itself away.
        self.raise();
    }
}

/// Actions a reader asks for — moving focus, invoking a control — are not
/// routed anywhere.
///
/// clonk-org/clonk-rs#392 is scoped to publishing semantics; acting on a
/// request would move the startup screen's own focus, which is input
/// behaviour, and belongs with the control model rather than with the bridge.
struct UnroutedActions;

impl ActionHandler for UnroutedActions {
    fn do_action(&mut self, _request: ActionRequest) {}
}

/// The AccessKit adapter for the application's carrier window.
pub(crate) struct WindowAccessibility {
    adapter: Adapter,
    published: PublishedWindow,
    tree_requested: TreeRequest,
}

impl WindowAccessibility {
    /// Attach an adapter to `window`, which must not have been shown yet —
    /// AccessKit panics otherwise, so the caller creates the window invisible
    /// and shows it once this has returned.
    pub(crate) fn attach(event_loop: &ActiveEventLoop, window: &Window) -> Self {
        let tree_requested = TreeRequest::default();
        let adapter = Adapter::with_direct_handlers(
            event_loop,
            window,
            tree_requested.clone(),
            UnroutedActions,
            tree_requested.clone(),
        );
        Self {
            adapter,
            published: PublishedWindow::default(),
            tree_requested,
        }
    }

    /// AccessKit needs every window event, before the application acts on it:
    /// focus and geometry changes are what tell a reader where the window is.
    pub(crate) fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.adapter.process_event(window, event);
    }

    /// Publish the current screen, if it says anything new.
    pub(crate) fn describe(&mut self, window: &Window, semantics: ScenSelSemantics) {
        if self.tree_requested.take() {
            self.published.invalidate();
        }
        if let Some(update) = self.published.update(semantics, &window.title()) {
            self.adapter.update_if_active(|| update);
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_frontend::accessibility::scen_sel_semantics;

    fn search_field() -> NodeId {
        node_id(Role::TextInput).expect("the search field is publishable")
    }

    fn status_node() -> NodeId {
        node_id(Role::Status).expect("the result status is publishable")
    }

    fn node(update: &TreeUpdate, id: NodeId) -> &PlatformNode {
        update
            .nodes
            .iter()
            .find_map(|(node_id, node)| (*node_id == id).then_some(node))
            .expect("the update should carry this node")
    }

    /// clonk-org/clonk-rs#392: a reader walking the window has to find the
    /// search field, with its role, its name and the text it currently holds.
    #[test]
    fn the_published_window_carries_the_search_field_with_its_name_and_text() {
        let update = window_tree_update(&scen_sel_semantics("cast", true, None, None), "Clonk");

        let window = node(&update, WINDOW);
        assert_eq!(window.children(), [search_field()]);

        let field = node(&update, search_field());
        assert_eq!(field.role(), PlatformRole::TextInput);
        assert_eq!(field.label(), Some("Scenario search"));
        assert_eq!(field.value(), Some("cast"));
    }

    /// clonk-org/clonk-rs#392: the result count has to reach a reader while
    /// the caret stays in the field, which is what a polite live region is.
    #[test]
    fn the_result_status_is_published_as_a_polite_live_region() {
        let update = window_tree_update(
            &scen_sel_semantics("cast", true, Some("3 of 40 scenarios"), None),
            "Clonk",
        );

        let status = node(&update, status_node());
        assert_eq!(status.value(), Some("3 of 40 scenarios"));
        assert_eq!(status.live(), Some(Live::Polite));
    }

    /// Focus is what a reader follows. It must name the field the screen has
    /// focused, and fall back to the window when nothing on screen has it —
    /// AccessKit requires a focus on every update.
    #[test]
    fn focus_names_the_focused_field_and_otherwise_the_window() {
        let focused = window_tree_update(&scen_sel_semantics("", true, None, None), "Clonk");
        assert_eq!(focused.focus, search_field());

        let unfocused = window_tree_update(&scen_sel_semantics("", false, None, None), "Clonk");
        assert_eq!(unfocused.focus, WINDOW);
    }

    /// Away from the scenario selector there is nothing to describe, and the
    /// window must not keep advertising a search field that is no longer
    /// drawn. The root still has to be published, because AccessKit's tree
    /// always needs one.
    #[test]
    fn a_screen_with_no_semantics_publishes_the_bare_window() {
        let update = window_tree_update(&ScenSelSemantics::default(), "Clonk");

        assert_eq!(update.nodes.len(), 1);
        let window = node(&update, WINDOW);
        assert_eq!(window.role(), PlatformRole::Window);
        assert_eq!(window.label(), Some("Clonk"));
        assert!(window.children().is_empty());
        assert_eq!(update.focus, WINDOW);
    }

    /// The screen is described once per event-loop turn, but the description
    /// only changes when the player does something. Re-sending an identical
    /// tree costs a reader real work — AccessKit still walks every node — so
    /// an unchanged screen must produce no update at all.
    #[test]
    fn an_unchanged_screen_is_not_published_a_second_time() {
        let mut published = PublishedWindow::default();
        let semantics = scen_sel_semantics("cast", true, Some("3 of 40 scenarios"), None);

        assert!(published.update(semantics.clone(), "Clonk").is_some());
        assert!(published.update(semantics, "Clonk").is_none());
    }

    /// Every part of the description a reader announces must be able to
    /// trigger an update on its own: the text typed, the result count, the
    /// no-result guidance, and which control has focus.
    #[test]
    fn each_announced_change_publishes_a_fresh_tree() {
        let mut published = PublishedWindow::default();
        published.update(
            scen_sel_semantics("cast", true, Some("3 of 40"), None),
            "Clonk",
        );

        for changed in [
            scen_sel_semantics("cast", false, Some("3 of 40"), None),
            scen_sel_semantics("castl", true, Some("3 of 40"), None),
            scen_sel_semantics("cast", true, Some("1 of 40"), None),
            scen_sel_semantics("cast", true, Some("3 of 40"), Some("No scenarios match.")),
        ] {
            let mut published = PublishedWindow::default();
            published.update(
                scen_sel_semantics("cast", true, Some("3 of 40"), None),
                "Clonk",
            );
            assert!(
                published.update(changed.clone(), "Clonk").is_some(),
                "{changed:?} should publish"
            );
        }
    }

    /// A reader that attaches after the window opened asks for the tree from
    /// scratch. Nothing on screen changed, so the dedup above would answer
    /// with silence and the reader would see the placeholder tree forever.
    #[test]
    fn a_reader_attaching_later_is_given_the_tree_again() {
        let mut published = PublishedWindow::default();
        let semantics = scen_sel_semantics("cast", true, None, None);
        published.update(semantics.clone(), "Clonk");

        published.invalidate();

        assert!(published.update(semantics, "Clonk").is_some());
    }

    /// The window title is part of what a reader announces, so a title change
    /// alone — the developer console adopting its own caption — republishes.
    #[test]
    fn a_retitled_window_publishes_a_fresh_tree() {
        let mut published = PublishedWindow::default();
        let semantics = scen_sel_semantics("", false, None, None);
        published.update(semantics.clone(), "Clonk");

        assert!(published
            .update(semantics, "Clonk Developer Mode")
            .is_some());
    }
}
