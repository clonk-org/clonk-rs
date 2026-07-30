//! The developer console's ordered edit selection.
//!
//! `C4EditCursor` owns one `C4ObjectList` selection (`C4EditCursor.h:35-52`)
//! and fans a single change notification to the property panel and the object
//! tree (`C4EditCursor.cpp:196-198`). Selection entries are added with
//! `C4ObjectList::stNone`, which appends at the tail
//! (`C4ObjectList.cpp:110-135`), so the list keeps insertion order rather than
//! any game ordering. `C4ObjectListDlg` writes back through the same owner
//! without feedback loops (`C4ObjectListDlg.cpp:599-646`).
//!
//! This module owns identity, ordering, mutation, pruning and notification
//! only. Pointer hit testing, editor gestures, overlays and dialog content stay
//! out (M10-P4-L082/L045).

use crate::ObjectId;

/// Which surface performed a write, so a subscriber can ignore its own echo
/// (`C4ObjectListDlg.cpp:599-646` guards against exactly this).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionWriter {
    /// The edit cursor in the viewport.
    EditCursor,
    /// The object-list tree.
    ObjectTree,
    /// The engine itself, pruning objects that no longer exist.
    Engine,
}

/// An immutable ordered view of the selection, emitted once per logical change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSnapshot {
    /// The selected objects in `stNone` insertion order.
    pub objects: Vec<ObjectId>,
    /// Increments once per logical mutation; a no-op does not advance it.
    pub revision: u64,
    /// The writer whose mutation produced this snapshot.
    pub writer: SelectionWriter,
}

/// The process-local ordered selection shared by the edit cursor, the property
/// panel and the object tree.
///
/// Every mutator returns `Some(snapshot)` exactly when the selection actually
/// changed, so a caller forwards at most one notification per logical action
/// and a no-op stays silent.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeveloperSelection {
    objects: Vec<ObjectId>,
    /// `C4EditCursor::Target` — the hovered object is a separate scalar and is
    /// deliberately not part of the selection (`C4EditCursor.h:39`).
    hover: Option<ObjectId>,
    revision: u64,
}

impl DeveloperSelection {
    pub fn new() -> Self {
        Self::default()
    }

    /// The selection in insertion order.
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn contains(&self, object: ObjectId) -> bool {
        self.objects.contains(&object)
    }

    /// The hovered object. Changing it never touches the selection or its
    /// revision, matching C++ keeping `Target` outside the list.
    pub fn hover(&self) -> Option<ObjectId> {
        self.hover
    }

    pub fn set_hover(&mut self, object: Option<ObjectId>) {
        self.hover = object;
    }

    /// `Selection.Clear(); Selection.Add(Target, stNone)`
    /// (`C4EditCursor.cpp:219`) — a plain click replaces the selection.
    pub fn replace(
        &mut self,
        writer: SelectionWriter,
        object: ObjectId,
    ) -> Option<SelectionSnapshot> {
        self.apply(writer, |objects| {
            if objects.as_slice() == [object] {
                return false;
            }
            objects.clear();
            objects.push(object);
            true
        })
    }

    /// `if (!Selection.Remove(Target)) Selection.Add(Target, stNone)`
    /// (`C4EditCursor.cpp:213-214`) — the ctrl-click toggle. Re-adding an
    /// object appends it at the tail, so order follows the last insertion.
    pub fn toggle(
        &mut self,
        writer: SelectionWriter,
        object: ObjectId,
    ) -> Option<SelectionSnapshot> {
        self.apply(writer, |objects| {
            match objects.iter().position(|entry| *entry == object) {
                Some(index) => {
                    objects.remove(index);
                }
                None => objects.push(object),
            }
            true
        })
    }

    /// `Selection.Add(object, stNone)` — append at the tail, never duplicating
    /// (`C4ObjectList.cpp:123-135`).
    pub fn append(
        &mut self,
        writer: SelectionWriter,
        object: ObjectId,
    ) -> Option<SelectionSnapshot> {
        self.apply(writer, |objects| {
            if objects.contains(&object) {
                return false;
            }
            objects.push(object);
            true
        })
    }

    /// A frame drag: clear, then append the framed objects in the order the
    /// caller enumerated them (`C4EditCursor.cpp:224`, then the drag-frame
    /// commit). Duplicates in `framed` collapse to their first position.
    pub fn select_frame(
        &mut self,
        writer: SelectionWriter,
        framed: impl IntoIterator<Item = ObjectId>,
    ) -> Option<SelectionSnapshot> {
        let mut selected: Vec<ObjectId> = Vec::new();
        for object in framed {
            if !selected.contains(&object) {
                selected.push(object);
            }
        }
        self.apply(writer, |objects| {
            if *objects == selected {
                return false;
            }
            *objects = selected;
            true
        })
    }

    /// `Selection.Clear()`.
    pub fn clear(&mut self, writer: SelectionWriter) -> Option<SelectionSnapshot> {
        self.apply(writer, |objects| {
            if objects.is_empty() {
                return false;
            }
            objects.clear();
            true
        })
    }

    /// Drops objects that no longer exist, keeping the survivors in order. The
    /// hovered scalar is pruned too, since C++'s `Target` is cleared alongside.
    pub fn prune(&mut self, is_live: impl Fn(ObjectId) -> bool) -> Option<SelectionSnapshot> {
        if self.hover.is_some_and(|hover| !is_live(hover)) {
            self.hover = None;
        }
        self.apply(SelectionWriter::Engine, |objects| {
            let before = objects.len();
            objects.retain(|object| is_live(*object));
            objects.len() != before
        })
    }

    /// The current selection as a snapshot, without advancing the revision.
    pub fn snapshot(&self, writer: SelectionWriter) -> SelectionSnapshot {
        SelectionSnapshot {
            objects: self.objects.clone(),
            revision: self.revision,
            writer,
        }
    }

    /// Runs one logical mutation, advancing the revision and emitting a
    /// snapshot only when it reported a change.
    fn apply(
        &mut self,
        writer: SelectionWriter,
        mutate: impl FnOnce(&mut Vec<ObjectId>) -> bool,
    ) -> Option<SelectionSnapshot> {
        mutate(&mut self.objects).then(|| {
            self.revision += 1;
            self.snapshot(writer)
        })
    }
}

/// One `C4ControlScript` the console's script input produces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionScriptControl {
    /// `ScriptCtrl.SetTargetObj(pObjects[i])` — the target changes per object.
    pub target: ObjectId,
    /// The scope stays `SCOPE_Global` for every object; only the target moves
    /// (`C4Control.cpp:935-943`).
    pub global_scope: bool,
}

/// `EMMO_Script`'s fan-out (`C4EditCursor.cpp:475`, `C4Control.cpp:932-944`).
///
/// One `C4ControlScript` is built once with `SCOPE_Global` and the console's
/// strictness, then **executed once per selected object** in selection order
/// with only its target re-pointed. An empty selection executes nothing at all
/// — C++ returns early on `!pObjects`, so the script is not run globally as a
/// fallback.
pub fn selection_script_controls(selection: &[ObjectId]) -> Vec<SelectionScriptControl> {
    selection
        .iter()
        .map(|target| SelectionScriptControl {
            target: *target,
            global_scope: true,
        })
        .collect()
}

/// The deferred property/object-list refresh (`C4EditCursor.cpp:80-86,196-199`).
///
/// `OnSelectionChanged` only raises `fSelectionChanged`; `Execute` consumes it
/// once per frame and *then* updates the property dialog and object list. Many
/// selection changes inside one frame therefore collapse into a single refresh.
///
/// There is no periodic refresh to pair this with. `PropertyDlg::Update` has
/// exactly five callers in the pinned source and every one is selection-driven;
/// `Tick35` never appears near the console. A tick-driven refresh would be an
/// invention.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PropertyRefresh {
    pending: bool,
}

impl PropertyRefresh {
    /// `C4EditCursor::OnSelectionChanged` — mark, do not refresh.
    pub fn mark_changed(&mut self) {
        self.pending = true;
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    /// `C4EditCursor::Execute`'s selection-update block: consumes the flag and
    /// reports whether this frame should refresh.
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(id: u64) -> ObjectId {
        ObjectId::new(id)
    }

    // C4EditCursor.cpp:213-224; C4ObjectList.cpp:110-135 — stNone appends, the
    // ctrl toggle removes or appends, and a frame keeps the enumerated order.
    #[test]
    fn developer_selection_preserves_toggle_frame_and_tree_order() {
        let mut selection = DeveloperSelection::new();

        // A plain click replaces (:219).
        let snapshot = selection
            .replace(SelectionWriter::EditCursor, object(7))
            .expect("selecting changes the selection");
        assert_eq!(snapshot.objects, vec![object(7)]);
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.writer, SelectionWriter::EditCursor);
        // Replacing with the same single object is a no-op and stays silent.
        assert_eq!(
            selection.replace(SelectionWriter::EditCursor, object(7)),
            None
        );
        assert_eq!(selection.revision(), 1);

        // Ctrl-toggle appends at the tail, never sorting (:213-214).
        selection.toggle(SelectionWriter::EditCursor, object(3));
        selection.toggle(SelectionWriter::EditCursor, object(9));
        assert_eq!(selection.objects(), [object(7), object(3), object(9)]);

        // Toggling an existing entry removes it without reordering survivors.
        selection.toggle(SelectionWriter::EditCursor, object(3));
        assert_eq!(selection.objects(), [object(7), object(9)]);
        // Re-adding puts it at the tail, following the last insertion.
        selection.toggle(SelectionWriter::EditCursor, object(3));
        assert_eq!(selection.objects(), [object(7), object(9), object(3)]);

        // Append never duplicates (C4ObjectList.cpp:123-135).
        assert_eq!(
            selection.append(SelectionWriter::ObjectTree, object(9)),
            None
        );
        assert_eq!(selection.objects(), [object(7), object(9), object(3)]);

        // A frame clears and takes the enumerated order, collapsing duplicates.
        let framed = selection
            .select_frame(
                SelectionWriter::EditCursor,
                [object(5), object(2), object(5), object(8)],
            )
            .expect("frame selection changes the selection");
        assert_eq!(framed.objects, vec![object(5), object(2), object(8)]);
        // Re-framing the identical set is a no-op.
        assert_eq!(
            selection.select_frame(
                SelectionWriter::EditCursor,
                [object(5), object(2), object(8)]
            ),
            None
        );

        // A tree-originated write carries its own writer, which is how a
        // subscriber suppresses its own echo (C4ObjectListDlg.cpp:599-646).
        let from_tree = selection
            .replace(SelectionWriter::ObjectTree, object(4))
            .expect("tree selection changes the selection");
        assert_eq!(from_tree.writer, SelectionWriter::ObjectTree);
        assert_eq!(from_tree.objects, vec![object(4)]);

        // Clearing an empty selection stays silent.
        selection.clear(SelectionWriter::EditCursor);
        assert!(selection.is_empty());
        assert_eq!(selection.clear(SelectionWriter::EditCursor), None);
    }

    // Removed objects are pruned without reordering survivors, and one logical
    // mutation notifies exactly once.
    #[test]
    fn developer_selection_prunes_removed_objects_and_notifies_once() {
        let mut selection = DeveloperSelection::new();
        selection.select_frame(
            SelectionWriter::EditCursor,
            [object(1), object(2), object(3), object(4)],
        );
        let revision_before = selection.revision();

        // Objects 2 and 4 are gone; 1 and 3 keep their relative order.
        let pruned = selection
            .prune(|object| object != self::object(2) && object != self::object(4))
            .expect("pruning changed the selection");
        assert_eq!(pruned.objects, vec![object(1), object(3)]);
        assert_eq!(pruned.writer, SelectionWriter::Engine);
        // Exactly one revision for the whole prune, not one per removal.
        assert_eq!(pruned.revision, revision_before + 1);
        assert_eq!(selection.revision(), revision_before + 1);

        // Pruning again with nothing to drop is silent and does not advance.
        assert_eq!(selection.prune(|_| true), None);
        assert_eq!(selection.revision(), revision_before + 1);

        // The hovered object is a separate scalar: setting it never advances
        // the revision or touches the selection (C4EditCursor.h:39).
        selection.set_hover(Some(object(3)));
        assert_eq!(selection.revision(), revision_before + 1);
        assert_eq!(selection.objects(), [object(1), object(3)]);

        // ...but a hovered object that disappears is pruned with the rest.
        selection.set_hover(Some(object(99)));
        selection.prune(|object| object != self::object(99));
        assert_eq!(selection.hover(), None);
        assert_eq!(
            selection.objects(),
            [object(1), object(3)],
            "pruning an unselected hover must not disturb the selection"
        );

        // An unknown object is pruned the same way a removed one is.
        selection.append(SelectionWriter::ObjectTree, object(42));
        let dropped = selection
            .prune(|object| object != self::object(42))
            .expect("an unknown object is pruned");
        assert_eq!(dropped.objects, vec![object(1), object(3)]);
    }

    // C4Control.cpp:932-944 and C4EditCursor.cpp:80-86,196-199 — the script
    // fan-out and the coalesced refresh.
    #[test]
    fn script_input_fans_out_over_the_selection_and_refresh_is_coalesced() {
        // One control per selected object, in selection order, all still
        // SCOPE_Global — only the target moves.
        let selection = [object(7), object(2), object(9)];
        let controls = selection_script_controls(&selection);
        assert_eq!(
            controls.iter().map(|c| c.target).collect::<Vec<_>>(),
            selection.to_vec()
        );
        assert!(controls.iter().all(|control| control.global_scope));

        // An empty selection executes nothing — C++ returns on `!pObjects`
        // rather than falling back to one global run.
        assert!(selection_script_controls(&[]).is_empty());

        // OnSelectionChanged only marks; Execute consumes it once.
        let mut refresh = PropertyRefresh::default();
        assert!(!refresh.pending());
        assert!(!refresh.take(), "an idle frame refreshes nothing");
        refresh.mark_changed();
        assert!(refresh.pending());
        assert!(refresh.take());
        assert!(
            !refresh.take(),
            "the flag is consumed, so one change is one refresh"
        );

        // Several changes inside one frame collapse into a single refresh —
        // this is what stops a multi-object edit updating the panel per object.
        refresh.mark_changed();
        refresh.mark_changed();
        refresh.mark_changed();
        assert!(refresh.take());
        assert!(!refresh.take());
    }
}
