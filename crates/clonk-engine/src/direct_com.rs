//! The classic direct-control dispatch chain:
//! `C4Player::InCom` → `C4Player::DirectCom` → `C4Player::ObjectCom` →
//! `C4Object::DirectCom` (C4Player.cpp:1490-1554, 1453-1488, 1368-1390;
//! C4Object.cpp:3327-3557) plus the `ObjectCom*` per-procedure helpers
//! (C4ObjectCom.cpp). Coms are the raw C4Constants.h bytes (COM_Left=1 …)
//! with the COM_Single/COM_Double/release modifiers.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::action::{ActionProcedure, ScriptCallbackTarget};
use crate::command::{CommandData, CommandId, CommandMode, CommandOperation, CommandRequest};
use crate::compat;
use crate::control::{
    PlayerCommandControlData, PlayerSelectControlData, C4MN_ADJUST_POSITION,
    COM_CLEAR_PRESSED_COMS, COM_CONTENTS, COM_CURSOR_FIRST, COM_CURSOR_LAST, COM_CURSOR_LEFT,
    COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE,
    COM_MENU_DOWN, COM_MENU_ENTER, COM_MENU_ENTER_ALL, COM_MENU_FIRST, COM_MENU_LAST,
    COM_MENU_LEFT, COM_MENU_NAVIGATION1, COM_MENU_NAVIGATION2, COM_MENU_RIGHT, COM_MENU_SELECT,
    COM_MENU_SHOW_TEXT, COM_MENU_UP, COM_NONE, COM_RELEASE_FIRST, COM_RELEASE_LAST,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
    COM_WHEEL_DOWN, COM_WHEEL_UP,
};
use crate::math::{self, itofix};
use crate::player::CountedControlType;
#[cfg(test)]
use crate::Landscape;
use crate::{
    message, ocf, tolerate_script_error, C4Fixed, CommandDirection, CrewInfoLink, DefinitionId,
    Direction, Engine, EngineError, FixedVec2, MessageSpec, MouseDragCarryableCursor,
    MouseDragSource, ObjectEnterOutcome, ObjectId, PhysicalInfo, Value, Vector2,
    CATEGORY_MOUSE_SELECT,
};

/// `C4DoubleClick` (C4Constants.h:156): frames within which a repeated com
/// becomes a COM_Double, and after which a buffered com flushes as
/// COM_Single.
pub const C4_DOUBLE_CLICK: i32 = 10;

#[derive(Clone, Copy)]
enum PlayerObjectCommandMode {
    None,
    Set,
    Add,
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseWorldCursor {
    Crosshair,
    Dig { material: bool },
    Enter(ObjectId),
    Grab(ObjectId),
    Ungrab(ObjectId),
    Carryable(ObjectId),
    DigObject(ObjectId),
    Chop(ObjectId),
    Build(ObjectId),
    Select(ObjectId),
    Attack(ObjectId),
    JumpLeft,
    JumpRight,
}

/// The command-bearing cursor state retained by `C4MouseControl` between
/// target refills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseDoubleClickAction {
    Attack,
    Grab,
    Ungrab,
    Build,
    Chop,
    Enter,
    Get,
    Dig { material: bool },
}

const C4P_COMMAND_SET: i32 = 1;
const C4P_COMMAND_ADD: i32 = 2;
const C4P_COMMAND_APPEND: i32 = 4;
const C4P_COMMAND_RANGE: i32 = 8;

/// The live object data consumed by C4ObjectMenu's Activate/Get refill.
/// Both the ordinary engine path and the reentrant script-host preview feed
/// this same builder so nested ExecuteCommand observes the complete menu,
/// including rows and selection, before it returns.
#[derive(Clone)]
pub(crate) struct InternalObjectMenuObject {
    pub id: ObjectId,
    /// Runtime identity of this object's current `C4ObjectList` contents
    /// link. `Exit` followed by `Enter` creates a new link even when the
    /// object and final list slot are unchanged.
    pub contents_link_generation: u64,
    pub definition_id: String,
    /// Effective C4Object::GetName (CustomName -> crew info -> definition).
    pub name: String,
    pub category: i32,
    pub ocf: u32,
    pub contents: Vec<ObjectId>,
    pub active: bool,
}

#[derive(Clone)]
pub(crate) struct InternalObjectMenuDefinition {
    pub description: String,
    pub no_get: bool,
    pub collection_limit: i32,
}

pub(crate) trait InternalObjectMenuSource {
    type Error;

    fn current_menu(&self, object: ObjectId) -> Option<crate::ObjectMenuState>;
    fn object(&self, object: ObjectId) -> Option<InternalObjectMenuObject>;
    fn definition(&self, definition: &str) -> Option<InternalObjectMenuDefinition>;
    fn object_menu_picture_snapshot(
        &self,
        object: ObjectId,
    ) -> Option<crate::ObjectMenuPictureSnapshot>;
    fn can_concat_picture_with(&self, object: ObjectId, other: ObjectId) -> bool;
    fn activate_value(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        container: ObjectId,
        menu_before_value: &crate::ObjectMenuState,
    ) -> Result<i32, Self::Error>;
    fn reject_collection(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        menu_before_call: &crate::ObjectMenuState,
    ) -> Result<bool, Self::Error>;
}

struct InternalObjectMenuPictureGroup {
    representative: ObjectId,
    count: i32,
}

fn internal_object_menu_picture_groups<S: InternalObjectMenuSource>(
    source: &S,
    contents: &[ObjectId],
    category_mask: i32,
) -> Vec<InternalObjectMenuPictureGroup> {
    let links = internal_object_menu_links(source, contents);
    let mut p_curr = InternalObjectMenuSafeCursor::before_start(None);
    let mut p_curr_id = if links.is_empty() {
        InternalObjectMenuSafeCursor::end(None)
    } else {
        InternalObjectMenuSafeCursor::at(&links, 0, None)
    };
    let mut groups = Vec::new();
    while let Some((seed, count)) = internal_object_menu_iterator_next(
        source,
        &links,
        &mut p_curr,
        &mut p_curr_id,
        category_mask,
    ) {
        let Some(seed_object) = source.object(seed) else {
            continue;
        };
        let representative = if seed_object.ocf & crate::ocf::FULL_CON == 0 {
            contents
                .iter()
                .filter_map(|candidate| source.object(*candidate))
                .find(|candidate| {
                    candidate.active
                        && candidate.definition_id == seed_object.definition_id
                        && candidate.ocf & crate::ocf::FULL_CON != 0
                })
                // Contents.Find returns once. Only that first full-con
                // candidate is tested for picture concatenation.
                .filter(|candidate| source.can_concat_picture_with(candidate.id, seed))
                .map(|candidate| candidate.id)
                .unwrap_or(seed)
        } else {
            seed
        };
        groups.push(InternalObjectMenuPictureGroup {
            representative,
            count,
        });
    }
    groups
}

fn internal_live_contents_definition_count<S: InternalObjectMenuSource>(
    source: &S,
    contents: &[ObjectId],
    definition_id: &str,
) -> i32 {
    let count = contents
        .iter()
        .filter_map(|candidate| source.object(*candidate))
        .filter(|candidate| candidate.active && candidate.definition_id == definition_id)
        .count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn internal_live_contents_count<S: InternalObjectMenuSource>(
    source: &S,
    contents: &[ObjectId],
) -> i32 {
    let count = contents
        .iter()
        .filter_map(|candidate| source.object(*candidate))
        .filter(|candidate| candidate.active)
        .count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn internal_refilled_object_menu_selection(
    items: &[crate::ObjectMenuItem],
    previous_selection: Option<i32>,
    selected_definition: Option<&str>,
) -> i32 {
    if items.is_empty() {
        return -1;
    }
    let mut desired = previous_selection.unwrap_or(-1);
    if let (Some(previous), Some(selected)) = (previous_selection, selected_definition) {
        if usize::try_from(previous)
            .ok()
            .and_then(|selection| items.get(selection))
            .is_some_and(|item| item.item_id == selected)
        {
            desired = previous;
        } else if let Some(selection) = items
            .iter()
            .position(|item| item.item_id == selected)
            .and_then(|selection| i32::try_from(selection).ok())
        {
            desired = selection;
        }
    } else if let Some(selection) = selected_definition
        .and_then(|selected| items.iter().position(|item| item.item_id == selected))
        .and_then(|selection| i32::try_from(selection).ok())
    {
        desired = selection;
    }
    if usize::try_from(desired)
        .ok()
        .and_then(|selection| items.get(selection))
        .is_some_and(|item| item.selectable)
    {
        return desired;
    }

    let mut below = desired
        .saturating_sub(1)
        .min(i32::try_from(items.len().saturating_sub(1)).unwrap_or(i32::MAX));
    while below >= 0 {
        if items
            .get(usize::try_from(below).unwrap_or(usize::MAX))
            .is_some_and(|item| item.selectable)
        {
            return below;
        }
        below -= 1;
    }
    let mut above = desired.saturating_add(1).max(0);
    while let Some(item) = usize::try_from(above)
        .ok()
        .and_then(|selection| items.get(selection))
    {
        if item.selectable {
            return above;
        }
        above = above.saturating_add(1);
    }
    -1
}

fn internal_object_menu_selected_definition(menu: &crate::ObjectMenuState) -> Option<String> {
    usize::try_from(menu.selection)
        .ok()
        .and_then(|selection| menu.items.get(selection))
        // C4ObjectMenu::checkIDSelection explicitly skips C4ID_None.
        .filter(|item| item.item_id != "NONE")
        .map(|item| item.item_id.clone())
}

fn activate_menu_state(
    crew_id: ObjectId,
    container_id: ObjectId,
    container_definition_id: &str,
    container_name: &str,
    refill_object_contents_count: i32,
    items: Vec<crate::ObjectMenuItem>,
    selection: i32,
) -> crate::ObjectMenuState {
    crate::ObjectMenuState {
        caption: format!("{} is empty.", container_name),
        symbol_id: container_definition_id.to_string(),
        title_symbol: crate::ObjectMenuSymbol::default(),
        identification: Value::Int(6),
        style: 0,
        equal_item_height: false,
        permanent: true,
        location: None,
        runtime_id: next_internal_object_menu_refill_token(),
        extra: crate::ObjectMenuExtra::default(),
        extra_data: 0,
        internal_refill_token: 0,
        selection,
        user_menu: false,
        command_object: Some(crew_id),
        scenario_callbacks: false,
        refill_object: Some(container_id),
        refill_object_contents_count,
        location_reset_generation: 0,
        items,
        columns: 5,
        lines: 0,
        text_progressing: false,
        decoration: None,
    }
}

static NEXT_INTERNAL_OBJECT_MENU_REFILL_TOKEN: AtomicU64 = AtomicU64::new(1);

fn next_internal_object_menu_refill_token() -> u64 {
    loop {
        let token = NEXT_INTERNAL_OBJECT_MENU_REFILL_TOKEN.fetch_add(1, Ordering::Relaxed);
        if token != 0 {
            return token;
        }
    }
}

pub(crate) fn next_object_menu_runtime_id() -> u64 {
    next_internal_object_menu_refill_token()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct InternalObjectMenuLink {
    object: ObjectId,
    generation: u64,
}

struct InternalObjectMenuMutationTracker {
    token: u64,
    menu_object: ObjectId,
    menu_identity: u64,
    container: ObjectId,
    removed_successors: HashMap<InternalObjectMenuLink, Option<InternalObjectMenuLink>>,
}

thread_local! {
    static INTERNAL_OBJECT_MENU_MUTATION_TRACKERS: RefCell<Vec<InternalObjectMenuMutationTracker>> =
        const { RefCell::new(Vec::new()) };
}

struct InternalObjectMenuMutationGuard {
    token: u64,
}

impl InternalObjectMenuMutationGuard {
    fn begin(token: u64, menu_object: ObjectId, menu_identity: u64, container: ObjectId) -> Self {
        INTERNAL_OBJECT_MENU_MUTATION_TRACKERS.with(|trackers| {
            trackers
                .borrow_mut()
                .push(InternalObjectMenuMutationTracker {
                    token,
                    menu_object,
                    menu_identity,
                    container,
                    removed_successors: HashMap::new(),
                });
        });
        Self { token }
    }
}

fn internal_object_menu_has_enclosing_refill(
    token: u64,
    menu_object: ObjectId,
    menu_identity: u64,
) -> bool {
    INTERNAL_OBJECT_MENU_MUTATION_TRACKERS.with(|trackers| {
        trackers.borrow().iter().any(|tracker| {
            tracker.token != token
                && tracker.menu_object == menu_object
                && tracker.menu_identity == menu_identity
        })
    })
}

impl Drop for InternalObjectMenuMutationGuard {
    fn drop(&mut self) {
        INTERNAL_OBJECT_MENU_MUTATION_TRACKERS.with(|trackers| {
            let mut trackers = trackers.borrow_mut();
            if let Some(index) = trackers
                .iter()
                .rposition(|tracker| tracker.token == self.token)
            {
                trackers.remove(index);
            }
        });
    }
}

pub(crate) fn track_internal_object_menu_link_removal(
    container: ObjectId,
    object: ObjectId,
    generation: u64,
    successor: Option<(ObjectId, u64)>,
) {
    let link = InternalObjectMenuLink { object, generation };
    let successor =
        successor.map(|(object, generation)| InternalObjectMenuLink { object, generation });
    INTERNAL_OBJECT_MENU_MUTATION_TRACKERS.with(|trackers| {
        for tracker in trackers
            .borrow_mut()
            .iter_mut()
            .filter(|tracker| tracker.container == container)
        {
            tracker.removed_successors.entry(link).or_insert(successor);
        }
    });
}

fn internal_object_menu_removed_successor(
    token: u64,
    link: InternalObjectMenuLink,
) -> Option<Option<InternalObjectMenuLink>> {
    INTERNAL_OBJECT_MENU_MUTATION_TRACKERS.with(|trackers| {
        trackers
            .borrow()
            .iter()
            .find(|tracker| tracker.token == token)
            .and_then(|tracker| tracker.removed_successors.get(&link).copied())
    })
}

impl InternalObjectMenuLink {
    fn from_object(object: &InternalObjectMenuObject) -> Self {
        Self {
            object: object.id,
            generation: object.contents_link_generation,
        }
    }
}

#[derive(Clone, Debug)]
enum InternalObjectMenuIteratorPosition {
    BeforeStart,
    Link(InternalObjectMenuLink),
    End,
}

/// Removal-safe `C4ObjectList::iterator` cursor. When the pointed link is
/// removed, C++ advances the registered iterator to that link's successor;
/// `C4ObjectListIterator::GetNext` then increments once more. Capturing the
/// successor link identities (rather than object ids) also prevents a later
/// re-entry of the same object from retargeting the iterator.
#[derive(Clone, Debug)]
struct InternalObjectMenuSafeCursor {
    position: InternalObjectMenuIteratorPosition,
    successors: Vec<InternalObjectMenuLink>,
    tracker_token: Option<u64>,
}

impl InternalObjectMenuSafeCursor {
    fn before_start(tracker_token: Option<u64>) -> Self {
        Self {
            position: InternalObjectMenuIteratorPosition::BeforeStart,
            successors: Vec::new(),
            tracker_token,
        }
    }

    fn at(links: &[InternalObjectMenuLink], index: usize, tracker_token: Option<u64>) -> Self {
        Self {
            position: InternalObjectMenuIteratorPosition::Link(links[index]),
            successors: links[index + 1..].to_vec(),
            tracker_token,
        }
    }

    fn end(tracker_token: Option<u64>) -> Self {
        Self {
            position: InternalObjectMenuIteratorPosition::End,
            successors: Vec::new(),
            tracker_token,
        }
    }

    /// Resolve removals which occurred since the prior `GetNext`. A newly
    /// inserted link with the same object id has a different generation and
    /// therefore cannot alias the removed link.
    fn resolve(&mut self, links: &[InternalObjectMenuLink]) -> Option<usize> {
        match self.position {
            InternalObjectMenuIteratorPosition::BeforeStart
            | InternalObjectMenuIteratorPosition::End => None,
            InternalObjectMenuIteratorPosition::Link(link) => {
                let tracker_token = self.tracker_token;
                let mut cursor = link;
                let mut visited = HashSet::new();
                let mut index = links.iter().position(|candidate| *candidate == cursor);
                while index.is_none() && visited.insert(cursor) {
                    let Some(token) = tracker_token else {
                        break;
                    };
                    let Some(successor) = internal_object_menu_removed_successor(token, cursor)
                    else {
                        break;
                    };
                    let Some(successor) = successor else {
                        *self = Self::end(tracker_token);
                        return None;
                    };
                    cursor = successor;
                    index = links.iter().position(|candidate| *candidate == cursor);
                }
                let index = index.or_else(|| {
                    self.successors.iter().find_map(|successor| {
                        links.iter().position(|candidate| candidate == successor)
                    })
                });
                match index {
                    Some(index) => {
                        *self = Self::at(links, index, tracker_token);
                        Some(index)
                    }
                    None => {
                        *self = Self::end(tracker_token);
                        None
                    }
                }
            }
        }
    }
}

/// A removal-safe forward C4ObjectList iterator for engine paths whose
/// callbacks can unlink and reinsert contents. The link generation keeps a
/// reinserted object distinct from the link that the registered C++ iterator
/// was pointing at before removal.
pub(crate) struct RemovalSafeContentsIterator {
    _mutation_tracker: InternalObjectMenuMutationGuard,
    cursor: InternalObjectMenuSafeCursor,
    advance_before_next: bool,
}

impl RemovalSafeContentsIterator {
    pub(crate) fn new(container: ObjectId, links: &[(ObjectId, u64)]) -> Self {
        let token = next_internal_object_menu_refill_token();
        let tracker = InternalObjectMenuMutationGuard::begin(token, container, 0, container);
        let links = links
            .iter()
            .map(|&(object, generation)| InternalObjectMenuLink { object, generation })
            .collect::<Vec<_>>();
        let cursor = if links.is_empty() {
            InternalObjectMenuSafeCursor::end(Some(token))
        } else {
            InternalObjectMenuSafeCursor::at(&links, 0, Some(token))
        };
        Self {
            _mutation_tracker: tracker,
            cursor,
            advance_before_next: false,
        }
    }

    pub(crate) fn next(&mut self, links: &[(ObjectId, u64)]) -> Option<ObjectId> {
        let links = links
            .iter()
            .map(|&(object, generation)| InternalObjectMenuLink { object, generation })
            .collect::<Vec<_>>();
        let index = self.cursor.resolve(&links)?;
        let index = if self.advance_before_next {
            let next = index.checked_add(1)?;
            if next >= links.len() {
                self.cursor = InternalObjectMenuSafeCursor::end(self.cursor.tracker_token);
                return None;
            }
            self.cursor = InternalObjectMenuSafeCursor::at(&links, next, self.cursor.tracker_token);
            next
        } else {
            index
        };
        self.advance_before_next = true;
        Some(links[index].object)
    }
}

fn internal_object_menu_links<S: InternalObjectMenuSource>(
    source: &S,
    contents: &[ObjectId],
) -> Vec<InternalObjectMenuLink> {
    contents
        .iter()
        .filter_map(|object| source.object(*object))
        .map(|object| InternalObjectMenuLink::from_object(&object))
        .collect()
}

/// One exact `C4ObjectListIterator::GetNext` step over a freshly read live
/// contents list (C4ObjectList.cpp:849-903). `p_curr` and `p_curr_id` are
/// separate registered iterators: removing a returned group head advances
/// both to its old successor, while duplicate suppression starts at the
/// independently advanced `p_curr_id`.
fn internal_object_menu_iterator_next<S: InternalObjectMenuSource>(
    source: &S,
    links: &[InternalObjectMenuLink],
    p_curr: &mut InternalObjectMenuSafeCursor,
    p_curr_id: &mut InternalObjectMenuSafeCursor,
    category_mask: i32,
) -> Option<(ObjectId, i32)> {
    let mut head_index = p_curr_id.resolve(links)?;
    let mut current_index = match p_curr.position {
        InternalObjectMenuIteratorPosition::BeforeStart => {
            *p_curr = p_curr_id.clone();
            head_index
        }
        InternalObjectMenuIteratorPosition::Link(_) => p_curr.resolve(links)?.checked_add(1)?,
        InternalObjectMenuIteratorPosition::End => return None,
    };

    // C4ObjectListIterator walks the retained list links directly. Unlike
    // C4ObjectList::Find/ObjectCount, GetNext does not test Obj->Status, so
    // AssignRemoval callbacks can still observe a Status-zero object until
    // the containing list actually unlinks it.
    let eligible = |index: usize| {
        source
            .object(links[index].object)
            .is_some_and(|object| category_mask == 0 || object.category & category_mask != 0)
    };
    while current_index < links.len() && !eligible(current_index) {
        current_index += 1;
    }
    if current_index == links.len() {
        *p_curr = InternalObjectMenuSafeCursor::end(p_curr.tracker_token);
        return None;
    }

    let current = source.object(links[current_index].object)?;
    let head = source.object(links[head_index].object)?;
    if current.definition_id != head.definition_id {
        *p_curr_id =
            InternalObjectMenuSafeCursor::at(links, current_index, p_curr_id.tracker_token);
        head_index = current_index;
    } else {
        // Preserve the literal C++ for-loop cursor behavior. After a match,
        // pCheck is assigned pCurrID and the loop increment advances it once,
        // so the newly selected candidate resumes checking at head+1.
        let mut check_index = head_index;
        while check_index < current_index {
            let current = source.object(links[current_index].object)?;
            if eligible(check_index)
                && source.can_concat_picture_with(links[check_index].object, current.id)
            {
                current_index += 1;
                while current_index < links.len() && !eligible(current_index) {
                    current_index += 1;
                }
                if current_index == links.len() {
                    *p_curr = InternalObjectMenuSafeCursor::end(p_curr.tracker_token);
                    return None;
                }
                let next = source.object(links[current_index].object)?;
                if next.definition_id != head.definition_id {
                    *p_curr_id = InternalObjectMenuSafeCursor::at(
                        links,
                        current_index,
                        p_curr_id.tracker_token,
                    );
                    head_index = current_index;
                    break;
                }
                check_index = head_index;
            }
            check_index += 1;
        }
    }

    let current = source.object(links[current_index].object)?;
    let mut count = 1i32;
    for candidate in links.iter().skip(current_index + 1) {
        let Some(candidate_object) = source.object(candidate.object) else {
            continue;
        };
        if candidate_object.definition_id != current.definition_id {
            break;
        }
        if (category_mask == 0 || candidate_object.category & category_mask != 0)
            && source.can_concat_picture_with(candidate.object, current.id)
        {
            count = count.saturating_add(1);
        }
    }

    *p_curr = InternalObjectMenuSafeCursor::at(links, current_index, p_curr.tracker_token);
    // Refresh the registered head iterator's successor chain at the instant
    // GetNext returns, before CalcValue may mutate the contents list.
    *p_curr_id = InternalObjectMenuSafeCursor::at(links, head_index, p_curr_id.tracker_token);
    Some((current.id, count))
}

pub(crate) fn build_activate_menu_state<S: InternalObjectMenuSource>(
    source: &mut S,
    crew_id: ObjectId,
    container_id: ObjectId,
    continue_existing: bool,
    reused_menu_identity: Option<u64>,
) -> Result<Option<crate::ObjectMenuState>, S::Error> {
    const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
    let continuing_menu = continue_existing
        .then(|| source.current_menu(crew_id))
        .flatten()
        .filter(|menu| {
            menu.identification == Value::Int(6) && menu.refill_object == Some(container_id)
        });
    let (previous_selection, selected_definition) = continuing_menu
        .as_ref()
        .map(|menu| {
            let selected_definition = internal_object_menu_selected_definition(menu);
            (Some(menu.selection), selected_definition)
        })
        .unwrap_or((None, None));
    let Some(container) = source.object(container_id) else {
        return Ok(None);
    };
    let Some(_container_definition) = source.definition(&container.definition_id) else {
        return Ok(None);
    };
    let container_name = container.name.clone();
    let contents = container.contents.clone();
    let refill_object_contents_count = if continuing_menu.is_some() {
        internal_live_contents_count(source, &contents)
    } else {
        0
    };
    let activate_category = crate::CATEGORY_STATIC_BACK
        | crate::CATEGORY_STRUCTURE
        | crate::CATEGORY_VEHICLE
        | crate::CATEGORY_OBJECT
        | CATEGORY_TRADE_LIVING;
    let mut menu = match continuing_menu {
        Some(mut menu) => {
            // C4ObjectMenu::DoRefillInternal only ClearItems(false)s an
            // existing Activate menu. Script-mutated size, decoration,
            // text/layout state, caption and symbols all survive.
            menu.items.clear();
            menu.refill_object_contents_count = refill_object_contents_count;
            menu
        }
        None => activate_menu_state(
            crew_id,
            container_id,
            &container.definition_id,
            &container_name,
            refill_object_contents_count,
            Vec::new(),
            previous_selection.unwrap_or(-1),
        ),
    };
    let menu_identity = source
        .current_menu(crew_id)
        .map(|menu| menu.internal_refill_token)
        .filter(|identity| *identity != 0)
        .or(reused_menu_identity)
        .unwrap_or_else(next_internal_object_menu_refill_token);
    let refill_token = next_internal_object_menu_refill_token();
    menu.internal_refill_token = menu_identity;

    let _mutation_tracker =
        InternalObjectMenuMutationGuard::begin(refill_token, crew_id, menu_identity, container_id);
    let initial_links = internal_object_menu_links(source, &contents);
    let mut p_curr = InternalObjectMenuSafeCursor::before_start(Some(refill_token));
    let mut p_curr_id = if initial_links.is_empty() {
        InternalObjectMenuSafeCursor::end(Some(refill_token))
    } else {
        InternalObjectMenuSafeCursor::at(&initial_links, 0, Some(refill_token))
    };
    while let Some(live_container) = source.object(container_id) {
        let live_contents = live_container.contents;
        let live_links = internal_object_menu_links(source, &live_contents);
        let Some((seed, count)) = internal_object_menu_iterator_next(
            source,
            &live_links,
            &mut p_curr,
            &mut p_curr_id,
            activate_category,
        ) else {
            break;
        };
        let Some(seed_object) = source.object(seed) else {
            continue;
        };
        let item_id = if seed_object.ocf & crate::ocf::FULL_CON == 0 {
            live_contents
                .iter()
                .copied()
                .filter_map(|candidate| source.object(candidate))
                .find(|object| {
                    object.active
                        && object.definition_id == seed_object.definition_id
                        && object.ocf & crate::ocf::FULL_CON != 0
                })
                // Contents.Find returns once. Only that first full-con
                // candidate is tested for picture concatenation.
                .filter(|candidate| source.can_concat_picture_with(candidate.id, seed))
                .map(|candidate| candidate.id)
                .unwrap_or(seed)
        } else {
            seed
        };
        let Some(item) = source.object(item_id) else {
            continue;
        };
        let Some(definition) = source.definition(&item.definition_id) else {
            continue;
        };
        if definition.no_get {
            continue;
        }
        let item_name = item.name.clone();
        let all_count =
            internal_live_contents_definition_count(source, &live_contents, &item.definition_id);
        let command = format!(
            "SetCommand(this,\"Activate\",Object({}))&&ExecuteCommand()",
            item_id.as_u64()
        );
        let command2 = format!(
            "SetCommand(this,\"Activate\", ,{},0,Object({}),{})&&ExecuteCommand()",
            all_count,
            container_id.as_u64(),
            item.definition_id
        );
        // C4ObjectMenu is already installed and frozen before GetValue.
        // Rows added by prior iterations and callback-side menu mutations
        // are therefore live at this exact call site.
        // C4ObjectMenu.cpp:194-199 captures Picture2Facet before GetValue
        // can mutate or remove the representative object.
        let picture_snapshot = source.object_menu_picture_snapshot(item_id);
        let value = source.activate_value(crew_id, item_id, container_id, &menu)?;
        menu = match source.current_menu(crew_id) {
            Some(live) if live.internal_refill_token == menu_identity => live,
            // A callback explicitly reopened/replaced the menu. Preserve
            // that live result instead of resurrecting the refill object.
            Some(replacement) => return Ok(Some(replacement)),
            None => return Ok(None),
        };
        let text_display_progress = if menu.text_progressing { 0 } else { -1 };
        menu.items.push(crate::ObjectMenuItem {
            caption: format!("Activate {}", item_name),
            info_caption: crate::normalize_menu_info_caption(definition.description),
            command,
            command2,
            count,
            item_id: item.definition_id,
            symbol: crate::ObjectMenuSymbol::default(),
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot,
            picture_object: Some(item_id),
            components: Vec::new(),
            selectable: true,
            value: Some(value),
            text_display_progress,
        });
    }

    menu.selection = internal_refilled_object_menu_selection(
        &menu.items,
        Some(menu.selection),
        selected_definition.as_deref(),
    );
    if !internal_object_menu_has_enclosing_refill(refill_token, crew_id, menu_identity) {
        menu.internal_refill_token = 0;
    }
    Ok(Some(menu))
}

pub(crate) fn build_container_contents_menu_state<S: InternalObjectMenuSource>(
    source: &mut S,
    crew_id: ObjectId,
    container_id: ObjectId,
    identification: i32,
    continue_existing: bool,
    reused_menu_identity: Option<u64>,
) -> Result<Option<crate::ObjectMenuState>, S::Error> {
    const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
    let continuing_menu = continue_existing
        .then(|| source.current_menu(crew_id))
        .flatten()
        .filter(|menu| {
            menu.identification == Value::Int(identification)
                && menu.refill_object == Some(container_id)
        });
    let (previous_selection, selected_definition) = continuing_menu
        .as_ref()
        .map(|menu| {
            let selected_definition = internal_object_menu_selected_definition(menu);
            (Some(menu.selection), selected_definition)
        })
        .unwrap_or((None, None));
    let Some(container) = source.object(container_id) else {
        return Ok(None);
    };
    let Some(_container_definition) = source.definition(&container.definition_id) else {
        return Ok(None);
    };
    let contents = container.contents.clone();
    let refill_object_contents_count = if continuing_menu.is_some() {
        internal_live_contents_count(source, &contents)
    } else {
        0
    };
    let get_category = crate::CATEGORY_STATIC_BACK
        | crate::CATEGORY_STRUCTURE
        | crate::CATEGORY_VEHICLE
        | crate::CATEGORY_OBJECT
        | CATEGORY_TRADE_LIVING;
    let mut menu = match continuing_menu {
        Some(mut menu) => {
            menu.items.clear();
            menu.refill_object_contents_count = refill_object_contents_count;
            menu
        }
        None => crate::ObjectMenuState {
            caption: format!("{} is empty.", container.name),
            symbol_id: container.definition_id,
            title_symbol: crate::ObjectMenuSymbol::default(),
            identification: Value::Int(identification),
            style: 0,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            internal_refill_token: 0,
            selection: previous_selection.unwrap_or(-1),
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: Some(container_id),
            refill_object_contents_count,
            location_reset_generation: 0,
            items: Vec::new(),
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        },
    };
    let menu_identity = source
        .current_menu(crew_id)
        .map(|menu| menu.internal_refill_token)
        .filter(|identity| *identity != 0)
        .or(reused_menu_identity)
        .unwrap_or_else(next_internal_object_menu_refill_token);
    let refill_token = next_internal_object_menu_refill_token();
    menu.internal_refill_token = menu_identity;
    let _mutation_tracker =
        InternalObjectMenuMutationGuard::begin(refill_token, crew_id, menu_identity, container_id);
    let initial_links = internal_object_menu_links(source, &contents);
    let mut p_curr = InternalObjectMenuSafeCursor::before_start(Some(refill_token));
    let mut p_curr_id = if initial_links.is_empty() {
        InternalObjectMenuSafeCursor::end(Some(refill_token))
    } else {
        InternalObjectMenuSafeCursor::at(&initial_links, 0, Some(refill_token))
    };
    // C4ObjectMenu reuses this loop-local string and only overwrites it for
    // multi-count rows. Preserve the legacy stale secondary command on a
    // later singleton row (C4ObjectMenu.cpp:314-318).
    let mut command2 = String::new();
    while let Some(live_container) = source.object(container_id) {
        let live_contents = live_container.contents;
        let live_links = internal_object_menu_links(source, &live_contents);
        let Some((seed, count)) = internal_object_menu_iterator_next(
            source,
            &live_links,
            &mut p_curr,
            &mut p_curr_id,
            get_category,
        ) else {
            break;
        };
        let Some(seed_object) = source.object(seed) else {
            continue;
        };
        let item_id = if seed_object.ocf & crate::ocf::FULL_CON == 0 {
            live_contents
                .iter()
                .copied()
                .filter_map(|candidate| source.object(candidate))
                .find(|object| {
                    object.active
                        && object.definition_id == seed_object.definition_id
                        && object.ocf & crate::ocf::FULL_CON != 0
                })
                .filter(|candidate| source.can_concat_picture_with(candidate.id, seed))
                .map(|candidate| candidate.id)
                .unwrap_or(seed)
        } else {
            seed
        };
        let Some(item) = source.object(item_id) else {
            continue;
        };
        let item_definition_id = item.definition_id.clone();
        let pre_callback_item_name = item.name.clone();
        let Some(definition) = source.definition(&item_definition_id) else {
            continue;
        };
        if definition.no_get {
            continue;
        }
        let mut get = item.ocf & crate::ocf::CARRYABLE != 0;
        if identification == 18 {
            let at_collection_limit = source.object(crew_id).is_some_and(|crew| {
                source
                    .definition(&crew.definition_id)
                    .is_some_and(|definition| {
                        crate::collection_limit_reached(
                            definition.collection_limit,
                            usize::try_from(internal_live_contents_count(source, &crew.contents))
                                .unwrap_or(0),
                        )
                    })
            });
            let rejected = source.reject_collection(crew_id, item_id, &menu)?;
            menu = match source.current_menu(crew_id) {
                Some(live) if live.internal_refill_token == menu_identity => live,
                Some(replacement) => return Ok(Some(replacement)),
                None => return Ok(None),
            };
            if at_collection_limit || rejected {
                get = false;
            }
        }
        if source
            .object(container_id)
            .is_some_and(|container| container.ocf & crate::ocf::ENTRANCE == 0)
        {
            get = true;
        }
        // C4ObjectMenu.cpp:311-313 renders Picture2Facet before the row is
        // added; keep that surface independent of later object deletion.
        let picture_snapshot = source.object_menu_picture_snapshot(item_id);
        let all_count = source
            .object(container_id)
            .map(|container| {
                internal_live_contents_definition_count(
                    source,
                    &container.contents,
                    &item_definition_id,
                )
            })
            .unwrap_or(0);
        let command_name = if get { "Get" } else { "Activate" };
        let item_name = source
            .object(item_id)
            .map(|item| item.name)
            .unwrap_or(pre_callback_item_name);
        let command = format!(
            "SetCommand(this, \"{}\", Object({})) && ExecuteCommand()",
            command_name,
            item_id.as_u64()
        );
        if all_count > 1 {
            command2 = format!(
                "SetCommand(this, \"{}\", , {},0, Object({}), {}) && ExecuteCommand()",
                command_name,
                all_count,
                container_id.as_u64(),
                item_definition_id
            );
        }
        let text_display_progress = if menu.text_progressing { 0 } else { -1 };
        menu.items.push(crate::ObjectMenuItem {
            caption: format!("{} {}", command_name, item_name),
            info_caption: crate::normalize_menu_info_caption(definition.description),
            command,
            command2: command2.clone(),
            count,
            item_id: item_definition_id,
            symbol: crate::ObjectMenuSymbol::default(),
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot,
            picture_object: Some(item_id),
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress,
        });
    }
    menu.selection = internal_refilled_object_menu_selection(
        &menu.items,
        Some(menu.selection),
        selected_definition.as_deref(),
    );
    if !internal_object_menu_has_enclosing_refill(refill_token, crew_id, menu_identity) {
        menu.internal_refill_token = 0;
    }
    Ok(Some(menu))
}

struct EngineInternalObjectMenuSource<'a>(&'a mut Engine);

impl InternalObjectMenuSource for EngineInternalObjectMenuSource<'_> {
    type Error = EngineError;

    fn current_menu(&self, object: ObjectId) -> Option<crate::ObjectMenuState> {
        self.0
            .find_object_index(object)
            .and_then(|index| self.0.objects[index].state.menu.clone())
    }

    fn object(&self, object: ObjectId) -> Option<InternalObjectMenuObject> {
        let index = self.0.find_object_index(object)?;
        let object = &self.0.objects[index];
        Some(InternalObjectMenuObject {
            id: object.id,
            contents_link_generation: object.state.contents_link_generation,
            definition_id: object.definition_id.clone(),
            name: object
                .state
                .custom_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    self.0
                        .crew_object_infos
                        .get(&object.id)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    self.0
                        .definitions
                        .get(&object.definition_id)
                        .map(|definition| definition.name().to_string())
                })
                .unwrap_or_else(|| object.definition_id.clone()),
            category: object.state.category,
            ocf: object.state.ocf,
            contents: object.state.contents.clone(),
            active: !object.destroyed && object.state.status != crate::ObjectStatus::Deleted,
        })
    }

    fn object_menu_picture_snapshot(
        &self,
        object: ObjectId,
    ) -> Option<crate::ObjectMenuPictureSnapshot> {
        self.0.native_object_menu_picture_snapshot(object)
    }

    fn definition(&self, definition: &str) -> Option<InternalObjectMenuDefinition> {
        self.0
            .definitions
            .get(definition)
            .map(|definition| InternalObjectMenuDefinition {
                description: definition.description().unwrap_or_default().to_string(),
                no_get: definition.no_get(),
                collection_limit: definition.collection_limit(),
            })
    }

    fn can_concat_picture_with(&self, object: ObjectId, other: ObjectId) -> bool {
        let Some(object) = self.0.object_snapshot(object) else {
            return false;
        };
        let Some(other) = self.0.object_snapshot(other) else {
            return false;
        };
        self.0.can_concat_picture_with(&object, &other)
    }

    fn activate_value(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        container: ObjectId,
        menu_before_value: &crate::ObjectMenuState,
    ) -> Result<i32, Self::Error> {
        let Some(command_index) = self.0.find_object_index(command_object) else {
            return Ok(0);
        };
        self.0.objects[command_index].state.menu = Some(menu_before_value.clone());
        self.0
            .object_value_in_container_for_menu(command_object, object, container, -1)
    }

    fn reject_collection(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        menu_before_call: &crate::ObjectMenuState,
    ) -> Result<bool, Self::Error> {
        let Some(command_index) = self.0.find_object_index(command_object) else {
            return Ok(false);
        };
        self.0.objects[command_index].state.menu = Some(menu_before_call.clone());
        let Some(object_index) = self.0.find_object_index(object) else {
            return Ok(false);
        };
        let definition_id = self.0.objects[object_index].definition_id.clone();
        let result = tolerate_script_error(self.0.call_object_function(
            command_index,
            "RejectCollect",
            vec![
                Value::C4Id(definition_id),
                compat::object_reference_value(object),
            ],
        ))?;
        Ok(result.is_some_and(|value| compat::value_raw_truthy(&value)))
    }
}

/// Backing selected by the one `C4Object::GetPhysical()` call at
/// ObjectComDigDouble entry. C++ retains this pointer across Activate:
/// mutations of temporary/info/definition storage remain visible, while a
/// fair-crew pointer keeps targeting its already-filled cached projection.
/// Switching temporary mode or changing definition does not retarget it.
#[derive(Clone)]
enum DigDoublePhysicalBacking {
    Temporary,
    FairCrew(PhysicalInfo),
    Info(PhysicalInfo),
    Definition(String),
}

/// `ComName(byCom)` (C4ObjectCom.cpp:800-852) for raw com bytes; feeds the
/// `Control{}`/`Contained{}` script callback names.
pub(crate) fn com_name_raw(com: u8) -> &'static str {
    const S: u8 = COM_SINGLE;
    const D: u8 = COM_DOUBLE;
    const R: u8 = COM_RELEASE_OFFSET;
    match com {
        COM_UP => "Up",
        c if c == COM_UP | S => "UpSingle",
        c if c == COM_UP | D => "UpDouble",
        c if c == COM_UP + R => "UpReleased",
        COM_DOWN => "Down",
        c if c == COM_DOWN | S => "DownSingle",
        c if c == COM_DOWN | D => "DownDouble",
        c if c == COM_DOWN + R => "DownReleased",
        COM_LEFT => "Left",
        c if c == COM_LEFT | S => "LeftSingle",
        c if c == COM_LEFT | D => "LeftDouble",
        c if c == COM_LEFT + R => "LeftReleased",
        COM_RIGHT => "Right",
        c if c == COM_RIGHT | S => "RightSingle",
        c if c == COM_RIGHT | D => "RightDouble",
        c if c == COM_RIGHT + R => "RightReleased",
        COM_DIG => "Dig",
        c if c == COM_DIG | S => "DigSingle",
        c if c == COM_DIG | D => "DigDouble",
        c if c == COM_DIG + R => "DigReleased",
        COM_THROW => "Throw",
        c if c == COM_THROW | S => "ThrowSingle",
        c if c == COM_THROW | D => "ThrowDouble",
        c if c == COM_THROW + R => "ThrowReleased",
        COM_SPECIAL => "Special",
        c if c == COM_SPECIAL | S => "SpecialSingle",
        c if c == COM_SPECIAL | D => "SpecialDouble",
        c if c == COM_SPECIAL + R => "SpecialReleased",
        COM_SPECIAL2 => "Special2",
        c if c == COM_SPECIAL2 | S => "Special2Single",
        c if c == COM_SPECIAL2 | D => "Special2Double",
        c if c == COM_SPECIAL2 + R => "Special2Released",
        COM_WHEEL_UP => "WheelUp",
        COM_WHEEL_DOWN => "WheelDown",
        COM_CURSOR_LEFT => "CursorLeft",
        c if c == COM_CURSOR_LEFT | S => "CursorLeftSingle",
        c if c == COM_CURSOR_LEFT | D => "CursorLeftDouble",
        c if c == COM_CURSOR_LEFT + R => "CursorLeftReleased",
        COM_CURSOR_TOGGLE => "CursorToggle",
        c if c == COM_CURSOR_TOGGLE | S => "CursorToggleSingle",
        c if c == COM_CURSOR_TOGGLE | D => "CursorToggleDouble",
        c if c == COM_CURSOR_TOGGLE + R => "CursorToggleReleased",
        COM_CURSOR_RIGHT => "CursorRight",
        c if c == COM_CURSOR_RIGHT | S => "CursorRightSingle",
        c if c == COM_CURSOR_RIGHT | D => "CursorRightDouble",
        c if c == COM_CURSOR_RIGHT + R => "CursorRightReleased",
        _ => "Undefined",
    }
}

/// `Coms2ComDir(iComs)` (C4ObjectCom.cpp:903-920): only the listed
/// direction-bit combinations map; everything else is COMD_Stop.
pub(crate) fn coms_to_com_dir(coms: i32) -> CommandDirection {
    let dir_coms = (1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_UP) | (1 << COM_DOWN);
    let up = 1 << COM_UP;
    let down = 1 << COM_DOWN;
    let left = 1 << COM_LEFT;
    let right = 1 << COM_RIGHT;
    match coms & dir_coms {
        c if c == up => CommandDirection::Up,
        c if c == up | right => CommandDirection::UpRight,
        c if c == right => CommandDirection::Right,
        c if c == down | right => CommandDirection::DownRight,
        c if c == down => CommandDirection::Down,
        c if c == down | left => CommandDirection::DownLeft,
        c if c == left => CommandDirection::Left,
        c if c == up | left => CommandDirection::UpLeft,
        _ => CommandDirection::Stop,
    }
}

/// The verbatim `switch (byCom)` labels of C4Object::DirectCom.
const COM_DOWN_D: u8 = COM_DOWN | COM_DOUBLE;
const COM_DIG_S: u8 = COM_DIG | COM_SINGLE;
const COM_DIG_D: u8 = COM_DIG | COM_DOUBLE;
const COM_THROW_D: u8 = COM_THROW | COM_DOUBLE;

/// `SimFlight` (C4Movement.cpp:623-653): fixed-point frame integration with
/// sign-step pixel traversal and an inclusive density contact interval.
pub(crate) fn sim_flight_to_density(
    position: &mut FixedVec2,
    velocity: &mut FixedVec2,
    density_min: i32,
    density_max: i32,
    mut iterations: i32,
    gravity: crate::C4Fixed,
    width: i32,
    height: i32,
    density_at: &impl Fn(i32, i32) -> i32,
) -> bool {
    let mut x = crate::math::fixtoi(position.x);
    let mut y = crate::math::fixtoi(position.y);
    loop {
        if iterations == 0 {
            return false;
        }
        iterations = iterations.wrapping_sub(1);
        position.x += velocity.x;
        position.y += velocity.y;
        let target_x = crate::math::fixtoi(position.x);
        let target_y = crate::math::fixtoi(position.y);
        if !(0..=width).contains(&target_x) || target_y >= height {
            return false;
        }

        let contact = loop {
            x += (target_x - x).signum();
            y += (target_y - y).signum();
            if (density_min..=density_max).contains(&density_at(x, y)) {
                break true;
            }
            if x == target_x && y == target_y {
                break false;
            }
        };
        velocity.y += gravity;
        if contact {
            *position = FixedVec2::from_ints(x, y);
            return true;
        }
    }
}

impl Engine {
    /// `C4ControlPlayerControl::Execute` (C4Control.cpp:386-395): count the
    /// original signed packet fields before the command is narrowed to the
    /// byte consumed by `C4Player::InCom`.
    #[doc(hidden)]
    pub fn execute_player_control(
        &mut self,
        owner: i32,
        command: i32,
        data: i32,
    ) -> Result<(), EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(());
        }
        if !(i32::from(COM_RELEASE_FIRST)..=i32::from(COM_RELEASE_LAST)).contains(&command) {
            let id = command.wrapping_mul(10_000).wrapping_add(data);
            self.count_player_control(owner, CountedControlType::DirectCom, id, 1);
        }
        self.player_in_com(owner, command as u8, data)
    }

    /// `C4ControlPlayerSelect::Execute` (C4Control.cpp:341-368): resolve the
    /// ordered raw object numbers, run MouseSelection callbacks, count the
    /// packet checksum, then replace the crew selection only when at least
    /// one crew object survived or the packet explicitly carried no objects.
    #[doc(hidden)]
    pub fn execute_player_select(
        &mut self,
        data: &PlayerSelectControlData,
    ) -> Result<bool, EngineError> {
        let owner = data.player;
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }

        let mut checksum = 0_i32;
        let mut selected = Vec::new();
        for number in &data.objects {
            let Some(object_id) = u64::try_from(*number)
                .ok()
                .map(ObjectId::new)
                .filter(|id| id.as_u64() != 0)
            else {
                continue;
            };
            let Some(index) = self.find_object_index(object_id).filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != crate::ObjectStatus::Deleted
            }) else {
                continue;
            };

            let live_number = i32::try_from(self.objects[index].id.as_u64()).unwrap_or(*number);
            checksum =
                checksum.wrapping_add(live_number.wrapping_mul(checksum.wrapping_add(4_787_821)));
            if self.objects[index].state.category & CATEGORY_MOUSE_SELECT != 0 {
                let _ = tolerate_script_error(self.call_object_function(
                    index,
                    "MouseSelection",
                    vec![Value::Int(owner)],
                ))?;
            }

            // The callback may remove the object or change the player's crew.
            if self.find_object_index(object_id).is_some_and(|index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != crate::ObjectStatus::Deleted
            }) && self.player_crew_roster(owner).contains(&object_id)
            {
                selected.push(object_id);
            }
        }

        self.count_player_control(owner, CountedControlType::Command, checksum, 1);

        if !selected.is_empty() || data.objects.is_empty() {
            self.player_unselect_crew(owner)?;
            for id in selected {
                let Some(index) = self.find_object_index(id).filter(|&index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != crate::ObjectStatus::Deleted
                }) else {
                    continue;
                };
                self.object_do_select(index, owner, false)?;
            }
            self.player_adjust_cursor_command(owner)?;
            if let Some(player) = self.players.get_mut(&owner) {
                player.control.cursor_selection = 0;
                player.control.cursor_toggled = 0;
                player.control.select_flash = 30;
            }
        }
        Ok(true)
    }

    /// `C4ControlPlayerCommand::Execute` (C4Control.cpp:413-426): count the
    /// raw packet checksum once, resolve both tolerant object-number pointers,
    /// then route through `C4Player::ObjectCommand` add-mode semantics.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn execute_player_command(
        &mut self,
        owner: i32,
        command: i32,
        x: i32,
        y: i32,
        target: i32,
        target2: i32,
        data: i32,
        add_mode: i32,
    ) -> Result<(), EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(());
        }

        let checksum = command
            .wrapping_add(x)
            .wrapping_add(y)
            .wrapping_add(target)
            .wrapping_add(target2);
        self.count_player_control(owner, CountedControlType::Command, checksum, 1);

        let Some(command) = CommandId::from_raw(command) else {
            return Ok(());
        };
        // C4GameObjects::ObjectPointer searches the active and inactive lists
        // without a Status check. The Rust object vector likewise retains
        // status-zero objects until detach; only an absent/nonpositive number
        // becomes nil here.
        let resolve = |engine: &Self, number: i32| {
            (number > 0)
                .then(|| ObjectId::new(number as u64))
                .filter(|id| engine.find_object_index(*id).is_some())
        };
        let target = resolve(self, target);
        let target2 = resolve(self, target2);
        let mode = if add_mode & C4P_COMMAND_APPEND != 0 {
            PlayerObjectCommandMode::Append
        } else if add_mode & C4P_COMMAND_ADD != 0 {
            PlayerObjectCommandMode::Add
        } else if add_mode & C4P_COMMAND_SET != 0 {
            PlayerObjectCommandMode::Set
        } else {
            PlayerObjectCommandMode::None
        };
        self.player_crew_object_command(
            owner,
            command,
            target,
            target2,
            x,
            y,
            data,
            mode,
            add_mode & C4P_COMMAND_RANGE != 0,
        )?;
        Ok(())
    }

    pub(crate) fn count_player_control(
        &mut self,
        owner: i32,
        control_type: CountedControlType,
        id: i32,
        count: i32,
    ) {
        // CountControl observes the cursor before InCom may switch it.
        let cursor = self.players.get(&owner).and_then(crate::Player::cursor);
        let new_action = self
            .players
            .get_mut(&owner)
            .is_some_and(|player| player.count_control(control_type, id, count));
        if !new_action {
            return;
        }
        let Some(cursor) = cursor.filter(|id| self.crew_object_infos.contains_key(id)) else {
            return;
        };
        self.count_crew_info_control(cursor, 1);
    }

    /// Increment one attached `C4ObjectInfo::ControlCount` once per native
    /// control point. Each resulting multiple of five invokes a distinct
    /// `DoExperience(1)` call (C4Command.cpp:1617-1622).
    pub(crate) fn count_crew_info_control(&mut self, object: ObjectId, gain: i32) {
        if gain <= 0 || !self.crew_object_infos.contains_key(&object) {
            return;
        }
        let Some(link) = self.crew_info_links.get(&object).copied() else {
            return;
        };
        for _ in 0..gain {
            let awards_experience = {
                let control_count = self.crew_info_control_counts.entry(link).or_default();
                *control_count = control_count.wrapping_add(1);
                *control_count % 5 == 0
            };
            if awards_experience {
                self.do_object_experience(object, 1);
            }
        }
    }

    /// Replay a synchronous host preview's runtime-only counter delta. Its
    /// corresponding experience calls are transported separately so rank,
    /// physical and presentation ordering stay identical to the preview.
    pub(crate) fn adjust_crew_info_control_count(&mut self, link: CrewInfoLink, gain: i32) {
        let control_count = self.crew_info_control_counts.entry(link).or_default();
        *control_count = control_count.wrapping_add(gain);
    }

    #[doc(hidden)]
    pub(crate) fn crew_info_control_count(&self, object: ObjectId) -> Option<i32> {
        let link = self.crew_info_links.get(&object)?;
        Some(
            self.crew_info_control_counts
                .get(link)
                .copied()
                .unwrap_or(0),
        )
    }

    /// `C4Player::InCom` (C4Player.cpp:1490-1554): pressed-com bookkeeping
    /// plus COM_Single/COM_Double synthesis around the LastCom buffer.
    pub fn player_in_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        // Coms for unknown players are dropped like C4Game control routing
        // does when Players.Get fails.
        if !self.players.contains_key(&owner) {
            return Ok(());
        }
        if com == COM_CLEAR_PRESSED_COMS {
            let player = self.player_mut(owner)?;
            player.control.pressed_coms = 0;
            player.control.last_com = i32::from(COM_NONE);
            return Ok(());
        }
        // Cursor menu ConvertCom (C4Player.cpp:1502-1508;
        // C4Menu.cpp:1040-1069). Only exact press coms convert: releases
        // remain raw and are discarded by the pressed-com guard below.
        let cursor_menu_active = self
            .crew_cursor(owner)
            .and_then(|cursor| self.find_object_index(cursor))
            .is_some_and(|index| self.objects[index].state.menu.is_some());
        let com = if cursor_menu_active {
            match com {
                COM_THROW => COM_MENU_ENTER,
                COM_DIG => COM_MENU_CLOSE,
                COM_SPECIAL2 => COM_MENU_ENTER_ALL,
                COM_UP => COM_MENU_UP,
                COM_LEFT => COM_MENU_LEFT,
                COM_DOWN => COM_MENU_DOWN,
                COM_RIGHT => COM_MENU_RIGHT,
                _ => com,
            }
        } else {
            com
        };
        // Menu control: no single/double processing (C4Player.cpp:1510-1513).
        if (COM_MENU_FIRST..=COM_MENU_LAST).contains(&com) {
            return self.player_direct_com(owner, com, data);
        }
        let mut com = com;
        if !(COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com) {
            // C4Player::ResetCursorView switches any target/scroll camera
            // back to cursor mode before dispatching a new press. Cursor
            // mode follows ViewCursor first, then Cursor, without changing
            // either logical pointer (C4Player.cpp:926-928,1518,1695-1712).
            self.player_mut(owner)?.reset_cursor_view();
            // Update state (C4Player.cpp:1520-1521).
            if (COM_RELEASE_FIRST - COM_RELEASE_OFFSET..=COM_RELEASE_LAST - COM_RELEASE_OFFSET)
                .contains(&com)
            {
                let player = self.player_mut(owner)?;
                player.control.pressed_coms |= 1 << com;
            }
            // Check LastCom buffer for prior COM_Single (C4Player.cpp:1522-1531).
            let (last_com, control_style) = {
                let player = self.player_mut(owner)?;
                (player.control.last_com, player.control.control_style)
            };
            if last_com != i32::from(COM_NONE) && last_com != i32::from(com) {
                // C++ stores LastCom as int32_t but DirectCom accepts uint8_t.
                // Preserve the full compiler word for comparisons and apply
                // the language-defined low-byte conversion only at dispatch.
                self.player_direct_com(owner, (last_com | i32::from(COM_SINGLE)) as u8, data)?;
                // AutoStopControl uses a single COM_Down instead of COM_Down_D
                // for drop (C4Player.cpp:1527-1530).
                if control_style && last_com == i32::from(COM_DOWN) {
                    self.player_mut(owner)?.control.last_com_down_double = C4_DOUBLE_CLICK;
                }
            }
            // Check LastCom buffer for COM_Double (C4Player.cpp:1532-1533).
            let player = self.player_mut(owner)?;
            if player.control.last_com == i32::from(com) {
                com |= COM_DOUBLE;
            }
            // Set before the DirectCom so scripts may clear it (:1534-1536).
            player.control.last_com = i32::from(com);
            player.control.last_com_delay = 0;
        } else {
            // KeyRelease: only when the press was registered (:1540-1548).
            let player = self.player_mut(owner)?;
            let bit = 1 << (com - COM_RELEASE_OFFSET);
            if player.control.pressed_coms & bit == 0 {
                return Ok(());
            }
            player.control.pressed_coms &= !bit;
        }
        // Pass regular/COM_Double byCom to player (:1550-1551).
        self.player_direct_com(owner, com, data)?;
        // LastComDownDouble process (:1552-1553).
        if com == COM_DOWN_D {
            self.player_mut(owner)?.control.last_com_down_double = C4_DOUBLE_CLICK;
        }
        Ok(())
    }

    /// The control half of `C4Player::Execute` (C4Player.cpp:242,
    /// 1215-1232): flash decrements, the LastCom COM_Single timeout and the
    /// LastComDownDouble countdown. Runs once per frame per player after
    /// object execution (C4Game.cpp:822 Players.Execute order).
    pub(crate) fn execute_player_controls(&mut self) -> Result<(), EngineError> {
        let mut owners: Vec<i32> = self.players.keys().copied().collect();
        owners.sort_unstable();
        for owner in owners {
            if self
                .players
                .get(&owner)
                .is_none_or(|player| player.status() == crate::PlayerStatus::Inactive)
            {
                continue;
            }
            self.execute_player_control_and_menu(owner)?;
            let _ = self.finish_player_execute_delays(owner);
        }
        Ok(())
    }

    /// The Tick1 control/menu/AutoContext portion of one C4Player::Execute.
    /// Callers perform UpdateCounts/UpdateView first and Tick35/delays after.
    pub(super) fn execute_player_control_and_menu(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        let timed_out_last_com = {
            let Some(player) = self.players.get_mut(&owner) else {
                return Ok(());
            };
            if player.status() == crate::PlayerStatus::Inactive {
                return Ok(());
            }
            // LastCom timeout (C4Player.cpp:1215-1229).
            if player.control.last_com != i32::from(COM_NONE) {
                player.control.last_com_delay += 1;
                if player.control.last_com_delay > C4_DOUBLE_CLICK {
                    Some(player.control.last_com)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(last_com) = timed_out_last_com {
            // C++ keeps LastCom visible during the synchronous COM_Single
            // callback and clears it only after DirectCom returns.
            if last_com & i32::from(COM_SINGLE) == 0 {
                self.player_direct_com(owner, (last_com | i32::from(COM_SINGLE)) as u8, 0)?;
            }
            if let Some(player) = self.players.get_mut(&owner) {
                player.control.last_com = i32::from(COM_NONE);
                player.control.last_com_delay = 0;
            }
        }
        if let Some(player) = self.players.get_mut(&owner) {
            // LastComDownDouble (C4Player.cpp:1231-1232).
            if player.control.last_com_down_double > 0 {
                player.control.last_com_down_double -= 1;
            }
        }
        self.refill_player_object_menu(owner)?;
        self.open_player_auto_context_menu(owner)?;
        Ok(())
    }

    /// Final delay tail of one C4Player::Execute. The return value mirrors the
    /// list-level retirement check, which runs only after every player has
    /// completed its Execute pass.
    pub(super) fn finish_player_execute_delays(&mut self, owner: i32) -> bool {
        let Some(player) = self.players.get_mut(&owner) else {
            return false;
        };
        if player.status() == crate::PlayerStatus::Inactive {
            return false;
        }
        player.advance_runtime_delays();
        let ready_to_retire = player.advance_retire_delay();
        if player.control.cursor_flash > 0 {
            player.control.cursor_flash -= 1;
        }
        if player.control.select_flash > 0 {
            player.control.select_flash -= 1;
        }
        ready_to_retire
    }

    /// Player-menu execution notices refill-target content-count changes
    /// after objects have run and performs the shared 35-tick refill
    /// (C4Player.cpp:206-212; C4ObjectMenu.cpp:448-459). Rebuild every
    /// refill-driven internal object menu before the AutoContextMenu tail.
    fn refill_player_object_menu(&mut self, owner: i32) -> Result<(), EngineError> {
        if self
            .players
            .get(&owner)
            .is_none_or(|player| player.status() == crate::PlayerStatus::Inactive)
        {
            return Ok(());
        }
        let periodic_refill = self.frame.is_multiple_of(35);
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(());
        };
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let Some(menu) = self.objects[crew_index].state.menu.as_ref() else {
            return Ok(());
        };
        let identification = match menu.identification {
            Value::Int(4) => 4,
            Value::Int(5) => 5,
            Value::Int(6) => 6,
            Value::Int(13) => 13,
            Value::Int(14) => 14,
            Value::Int(18) => 18,
            _ => return Ok(()),
        };
        let refill_object = menu.refill_object;
        let previous_count = menu.refill_object_contents_count;
        let previous_item_count = menu.items.len();
        let previous_style = menu.style;
        let previous_runtime_id = menu.runtime_id;
        let previous_location_reset_generation = menu.location_reset_generation;
        let context_has_command_object = menu.command_object.is_some();
        let Some(container_id) = refill_object else {
            if periodic_refill {
                let _ = self.close_object_menu(crew_id, true)?;
            }
            return Ok(());
        };
        let Some(container_index) = self.find_object_index(container_id) else {
            if periodic_refill {
                let _ = self.close_object_menu(crew_id, true)?;
            }
            return Ok(());
        };
        let current_count = self.live_contents_count(&self.objects[container_index].state.contents);
        if !periodic_refill && current_count == previous_count {
            return Ok(());
        }
        // DoRefillInternal reports failure for these mode-specific invalid
        // states; C4Menu::Execute then closes directly, without consulting a
        // user-menu cancellation callback (C4ObjectMenu.cpp:207-242,328-334;
        // C4Menu.cpp:990-999).
        let refill_fails = match identification {
            4 => {
                !self.base_buy_enabled
                    || !self
                        .players
                        .contains_key(&self.objects[container_index].state.base)
            }
            5 => !self.base_sell_enabled,
            14 => !context_has_command_object,
            _ => false,
        };
        if refill_fails {
            let _ = self.close_object_menu(crew_id, true)?;
            return Ok(());
        }
        match identification {
            4 => self.refill_base_buy_menu(crew_index, container_index)?,
            5 => self.refill_base_sell_menu(crew_index, container_index)?,
            6 => self.open_activate_menu(crew_index, container_index)?,
            13 | 18 => {
                self.open_container_contents_menu(crew_index, container_index, identification)?;
            }
            14 => self.refill_context_menu(crew_index, container_index, current_count)?,
            _ => unreachable!("filtered refill-driven object-menu id"),
        }
        // C4Menu::RefillInternal clears LocationSet only after a successful
        // DoRefillInternal, and only when the final count grows (or a Context
        // menu shrinks). Ordinary AddMenuItem writes do not reach this path;
        // ClearMenuItems(true) already carries its own generation marker.
        if let Some(menu) = self.objects[crew_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.runtime_id == previous_runtime_id)
        {
            let invalidates_location = menu.items.len() > previous_item_count
                || (menu.items.len() < previous_item_count && previous_style == 1);
            if invalidates_location
                && menu.location_reset_generation == previous_location_reset_generation
            {
                menu.mark_location_reset();
            }
        }
        // C4ObjectMenu::Execute stores the observed count before invoking
        // the inherited refill. Store it on the still-matching menu after
        // each non-Context helper returns; Context does the write inside its
        // token-guarded frozen refill so callback-replaced menus stay clean.
        if identification != 14 {
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(identification)
                    && menu.refill_object == Some(container_id)
            }) {
                menu.refill_object_contents_count = current_count;
            }
        }
        Ok(())
    }

    pub(super) fn open_player_auto_context_menu(&mut self, owner: i32) -> Result<(), EngineError> {
        let Some(crew_index) = self
            .crew_cursor(owner)
            .and_then(|crew| self.find_object_index(crew))
        else {
            return Ok(());
        };
        if !self
            .players
            .get(&owner)
            .is_some_and(|player| player.control.auto_context_menu)
            || !self.objects[crew_index].commands.is_empty()
            || self.objects[crew_index].state.menu.is_some()
            || !self.objects[crew_index].state.crew_member
        {
            return Ok(());
        }
        let Some(base_index) = self.objects[crew_index]
            .state
            .container
            .and_then(|base| self.find_object_index(base))
        else {
            return Ok(());
        };
        let auto_context = self
            .definitions
            .get(&self.objects[base_index].definition_id)
            .is_some_and(|definition| definition.auto_context_menu());
        if auto_context {
            self.open_context_menu(crew_index, base_index, true, None)?;
        }
        Ok(())
    }

    fn context_function_item(
        &self,
        function: &crate::ScriptContextFunction,
        caption: String,
        command: String,
        fallback_picture: Option<ObjectId>,
    ) -> crate::ObjectMenuItem {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let image = function
            .image
            .as_deref()
            .filter(|image| !image.is_empty())
            .unwrap_or("NONE");
        let fallback_picture = (image == "NONE" || !self.definitions.contains_key(image))
            .then_some(fallback_picture)
            .flatten();
        let fallback_snapshot = fallback_picture
            .and_then(|object| self.find_object_index(object))
            .map(|index| {
                let object = &self.objects[index];
                crate::ObjectMenuPictureSnapshot {
                    definition_id: object.definition_id.clone(),
                    symbol_size: 35,
                    base_graphics: object.state.base_graphics.clone(),
                    graphics_overlays: object.state.graphics_overlays.clone(),
                    blit_mode: object.state.blit_mode,
                    color: object.state.color,
                    color_modulation: object.state.color_modulation,
                    picture_rect: object.state.picture_rect,
                    rank: None,
                }
            });
        crate::ObjectMenuItem {
            caption,
            info_caption: crate::normalize_menu_info_caption(
                function.description.clone().unwrap_or_default(),
            ),
            command,
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id: "NONE".to_owned(),
            symbol: crate::ObjectMenuSymbol::Definition,
            image: if image != "NONE" && self.definitions.contains_key(image) {
                crate::ObjectMenuImage::Indexed {
                    index: function.image_phase,
                }
            } else if let Some(object) = fallback_picture {
                crate::ObjectMenuImage::Object { object }
            } else {
                crate::ObjectMenuImage::None
            },
            presentation_definition_id: (image != "NONE" && self.definitions.contains_key(image))
                .then(|| image.to_owned()),
            picture_snapshot: fallback_snapshot,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        }
    }

    fn add_native_context_menu_item(&mut self, menu_object: ObjectId, item: crate::ObjectMenuItem) {
        let Some(menu_index) = self.find_object_index(menu_object) else {
            return;
        };
        let Some(menu) = self.objects[menu_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.identification == Value::Int(14))
        else {
            return;
        };
        if menu.internal_refill_token == 0 && menu.selection == -1 && item.selectable {
            menu.selection = menu.items.len() as i32;
        }
        menu.items.push(item);
    }

    fn record_context_function_item(
        &mut self,
        menu_object: ObjectId,
        publish: bool,
        items: &mut Vec<crate::ObjectMenuItem>,
        item: crate::ObjectMenuItem,
    ) {
        if publish {
            self.add_native_context_menu_item(menu_object, item.clone());
        }
        items.push(item);
    }

    fn context_condition_on_object(
        &mut self,
        object_index: usize,
        function: &crate::ScriptContextFunction,
        arguments: &str,
        label: &str,
    ) -> Result<bool, EngineError> {
        let Some(condition) = function.condition.as_deref() else {
            return Ok(true);
        };
        let source = format!("{condition}({arguments})");
        Ok(
            tolerate_script_error(self.direct_exec_on_object(object_index, &source, label))?
                .is_some_and(|value| compat::value_raw_truthy(&value)),
        )
    }

    fn global_script_menu_functions(&self, prefix: &str) -> Vec<crate::ScriptContextFunction> {
        let Some(global_functions) = self.global_script_functions.as_deref() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        self.global_script_function_order
            .iter()
            .rev()
            .filter(|name| seen.insert(name.as_str()))
            .filter(|name| name.starts_with(prefix))
            .filter_map(|name| global_functions.get(name))
            .map(|function| {
                let mut metadata = crate::script_context_function_metadata(function);
                if metadata.condition.as_ref().is_some_and(|condition| {
                    !self.global_menu_condition_resolves(&function.name, condition)
                }) {
                    metadata.condition = None;
                }
                metadata
            })
            .collect()
    }

    /// All native classes of `C4ObjectMenu::AddContextFunctions`, in their
    /// C++ order: ActionContext, effect Fx*Context, AttachContext,
    /// Activate/ControlDigDouble, then target Context* functions
    /// (C4ObjectMenu.cpp:544-685).
    fn context_function_menu_items(
        &mut self,
        target_index: usize,
        menu_object: ObjectId,
        publish: bool,
    ) -> Result<Vec<crate::ObjectMenuItem>, EngineError> {
        let Some(menu_index) = self.find_object_index(menu_object) else {
            return Ok(Vec::new());
        };
        let target_id = self.objects[target_index].id;
        let target_definition = self.objects[target_index].definition_id.clone();
        let target_action = self.objects[target_index].state.action.clone();
        let target_action_active = self
            .definitions
            .get(&target_definition)
            .is_some_and(|definition| !definition.action_library().is_idle_state(&target_action));
        let target_action_target = self.objects[target_index].state.action.target;
        let mut items = Vec::new();

        // ActionContext functions of the target's first action target.
        if target_action_active {
            if let Some(action_target_index) =
                target_action_target.and_then(|id| self.find_object_index(id))
            {
                let action_target_id = self.objects[action_target_index].id;
                let definition_id = self.objects[action_target_index].definition_id.clone();
                let functions = self
                    .definitions
                    .get(&definition_id)
                    .map(|definition| definition.script_menu_functions("ActionContext"))
                    .unwrap_or_default();
                for function in functions {
                    let image = function.image.as_deref().unwrap_or("NONE");
                    let arguments = format!(
                        "Object({}), C4Id(\"{}\"), Object({})",
                        menu_object.as_u64(),
                        image,
                        target_id.as_u64()
                    );
                    if !self.context_condition_on_object(
                        action_target_index,
                        &function,
                        &arguments,
                        "ActionContextCondition",
                    )? {
                        continue;
                    }
                    let command = format!(
                        "ProtectedCall(Object({}),\"{}\",this,Object({}))",
                        action_target_id.as_u64(),
                        function.function,
                        target_id.as_u64()
                    );
                    let item = self.context_function_item(
                        &function,
                        function.label.clone(),
                        command,
                        None,
                    );
                    self.record_context_function_item(menu_object, publish, &mut items, item);
                }
            }
        }

        // Active effect context functions use the callback script selected
        // by the live command target object/id, or the global script host.
        let mut effect_cursor = None;
        loop {
            let effects = &self.objects[target_index].state.effects;
            let mut effect_index = crate::effect_frame_cursor_next_index(effects, effect_cursor);
            let Some(effect) = (loop {
                let Some(effect) = effects.get(effect_index).cloned() else {
                    break None;
                };
                effect_index += 1;
                if effect.priority > 0 {
                    break Some(effect);
                }
            }) else {
                break;
            };
            effect_cursor = Some(crate::EffectFrameCursor {
                number: effect.number,
                priority: effect.priority.unsigned_abs(),
            });
            let prefix = format!("Fx{}Context", effect.name);
            let command_target = effect.command_target.and_then(|number| {
                u64::try_from(number)
                    .ok()
                    .map(ObjectId::new)
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    })
            });
            let (functions, condition_object, callback_definition) =
                if let Some(command_index) = command_target {
                    let definition_id = self.objects[command_index].definition_id.clone();
                    if let Some(live_effect) = self.objects[target_index]
                        .state
                        .effects
                        .iter_mut()
                        .find(|live_effect| live_effect.number == effect.number)
                    {
                        // GetCallbackScript refreshes idCommandTarget while the
                        // object target is alive, preserving a definition
                        // fallback if a condition deletes that object.
                        live_effect.command_id = Some(definition_id.clone());
                    }
                    let functions = self
                        .definitions
                        .get(&definition_id)
                        .map(|definition| definition.script_menu_functions(&prefix))
                        .unwrap_or_default();
                    (functions, Some(command_index), Some(definition_id))
                } else if let Some(definition_id) = effect
                    .command_id
                    .clone()
                    .filter(|definition_id| self.definitions.contains_key(definition_id))
                {
                    let functions = self
                        .definitions
                        .get(&definition_id)
                        .map(|definition| definition.script_menu_functions(&prefix))
                        .unwrap_or_default();
                    (functions, None, Some(definition_id))
                } else {
                    (self.global_script_menu_functions(&prefix), None, None)
                };
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}),{},Object({}),C4Id(\"{}\")",
                    target_id.as_u64(),
                    effect.number,
                    menu_object.as_u64(),
                    image
                );
                let enabled = if let Some(command_index) = condition_object {
                    self.context_condition_on_object(
                        command_index,
                        &function,
                        &arguments,
                        "EffectContextCondition",
                    )?
                } else if let Some(definition_id) = callback_definition.as_deref() {
                    if let Some(condition) = function.condition.as_deref() {
                        let source = format!(
                            "DefinitionCall({}, \"{}\", {})",
                            definition_id, condition, arguments
                        );
                        tolerate_script_error(self.direct_exec_on_object(
                            menu_index,
                            &source,
                            "EffectContextCondition",
                        ))?
                        .is_some_and(|value| compat::value_raw_truthy(&value))
                    } else {
                        true
                    }
                } else if let Some(condition) = function.condition.as_deref() {
                    if let Some((script_name, script)) =
                        self.global_menu_callback_script(&function.function)
                    {
                        let source = format!("{condition}({arguments})");
                        let value = self.direct_exec_script_control_host(
                            &script_name,
                            script.as_ref(),
                            &source,
                            "EffectContextCondition",
                            None,
                        )?;
                        compat::value_raw_truthy(&value)
                    } else {
                        let source = format!("global->~{condition}({arguments})");
                        tolerate_script_error(self.direct_exec_on_object(
                            menu_index,
                            &source,
                            "EffectContextCondition",
                        ))?
                        .is_some_and(|value| compat::value_raw_truthy(&value))
                    }
                } else {
                    true
                };
                if !enabled {
                    continue;
                }
                let live_effect = self.objects[target_index]
                    .state
                    .effects
                    .iter()
                    .find(|live_effect| live_effect.number == effect.number)
                    .cloned();
                let live_command_target = live_effect
                    .as_ref()
                    .map_or(effect.command_target, |live_effect| {
                        live_effect.command_target
                    })
                    .and_then(|number| u64::try_from(number).ok())
                    .map(ObjectId::new)
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    });
                let live_definition = live_effect
                    .as_ref()
                    .and_then(|live_effect| live_effect.command_id.clone())
                    .filter(|definition_id| self.definitions.contains_key(definition_id))
                    .or_else(|| callback_definition.clone());
                let command = if let Some(command_index) = live_command_target {
                    let command_id = self.objects[command_index].id;
                    format!(
                        "ProtectedCall(Object({}),\"{}\",Object({}),{},Object({}),{})",
                        command_id.as_u64(),
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                } else if let Some(definition_id) = live_definition.as_deref() {
                    format!(
                        "DefinitionCall({}, \"{}\", Object({}),{},Object({}),{})",
                        definition_id,
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                } else {
                    format!(
                        "global->~{}(Object({}),{},Object({}),{})",
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                };
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }

        // AttachContext functions of every active DFA_ATTACH object whose
        // first action target is the context target, in global object order.
        let attached_objects = self.exec_list.iter().rev().copied().collect::<Vec<_>>();
        for attached_id in attached_objects {
            let Some(attached_index) = self.find_object_index(attached_id) else {
                continue;
            };
            let still_attached = {
                let object = &self.objects[attached_index];
                let action_active =
                    self.definitions
                        .get(&object.definition_id)
                        .is_some_and(|definition| {
                            !definition
                                .action_library()
                                .is_idle_state(&object.state.action)
                        });
                !object.destroyed
                    && object.state.status.is_active()
                    && object.state.action.target == Some(target_id)
                    && action_active
                    && self.object_procedure(attached_index) == ActionProcedure::Attach
            };
            if !still_attached {
                continue;
            }
            let definition_id = self.objects[attached_index].definition_id.clone();
            let functions = self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.script_menu_functions("AttachContext"))
                .unwrap_or_default();
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}), C4Id(\"{}\"), Object({})",
                    menu_object.as_u64(),
                    image,
                    target_id.as_u64()
                );
                if !self.context_condition_on_object(
                    attached_index,
                    &function,
                    &arguments,
                    "AttachContextCondition",
                )? {
                    continue;
                }
                let command = format!(
                    "ProtectedCall(Object({}),\"{}\",this,Object({}))",
                    attached_id.as_u64(),
                    function.function,
                    target_id.as_u64()
                );
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }

        // Exact Activate and ControlDigDouble rows, with the Context*
        // DescText duplicate scan performed independently for each row.
        for name in ["Activate", "ControlDigDouble"] {
            let eligible = if name == "Activate" {
                self.objects[target_index].state.container == Some(menu_object)
            } else {
                self.object_procedure(menu_index) == ActionProcedure::Push
                    && self.objects[menu_index].state.action.target == Some(target_id)
            };
            if !eligible {
                continue;
            }
            let Some(function) = self
                .definitions
                .get(&target_definition)
                .and_then(|definition| definition.script_menu_function(name))
            else {
                continue;
            };
            let image = function.image.as_deref().unwrap_or("NONE");
            let arguments = format!("Object({}), C4Id(\"{}\")", menu_object.as_u64(), image);
            if !self.context_condition_on_object(
                target_index,
                &function,
                &arguments,
                "ContextFunctionCondition",
            )? {
                continue;
            }
            let caption = if function.has_description {
                function.label.clone()
            } else {
                self.objects[target_index]
                    .state
                    .custom_name
                    .clone()
                    .or_else(|| {
                        self.crew_object_infos
                            .get(&target_id)
                            .map(|info| info.name.clone())
                    })
                    .or_else(|| {
                        self.definitions
                            .get(&target_definition)
                            .map(|definition| definition.name().to_owned())
                    })
                    .unwrap_or_else(|| target_definition.clone())
            };
            let duplicate_functions = self
                .definitions
                .get(&target_definition)
                .map(|definition| definition.script_menu_functions("Context"))
                .unwrap_or_default();
            let mut duplicate = false;
            for context in duplicate_functions {
                let context_image = context.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}), C4Id(\"{}\")",
                    menu_object.as_u64(),
                    context_image
                );
                if self.context_condition_on_object(
                    target_index,
                    &context,
                    &arguments,
                    "ContextFunctionCondition",
                )? && caption == context.label
                {
                    duplicate = true;
                }
            }
            if duplicate {
                continue;
            }
            let command = format!(
                "ProtectedCall(Object({}),\"{}\",this)",
                target_id.as_u64(),
                function.function
            );
            let item = self.context_function_item(&function, caption, command, Some(target_id));
            self.record_context_function_item(menu_object, publish, &mut items, item);
        }

        // Target Context* functions: crew members must be owned by the menu
        // object and living targets must still be alive.
        let target = &self.objects[target_index].state;
        let menu_owner = self.objects[menu_index].state.owner;
        if (target.ocf & ocf::CREW_MEMBER == 0 || target.owner == menu_owner)
            && (target.category & crate::CATEGORY_LIVING == 0 || target.alive)
        {
            let functions = self
                .definitions
                .get(&target_definition)
                .map(|definition| definition.script_menu_functions("Context"))
                .unwrap_or_default();
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!("Object({}), C4Id(\"{}\")", menu_object.as_u64(), image);
                if !self.context_condition_on_object(
                    target_index,
                    &function,
                    &arguments,
                    "ContextCondition",
                )? {
                    continue;
                }
                let command = format!(
                    "ProtectedCall(Object({}),\"{}\",this)",
                    target_id.as_u64(),
                    function.function
                );
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }
        Ok(items)
    }

    /// Internal C4MN_Context refill for an arbitrary target. Automatic
    /// contained menus are permanent; mouse C4CMD_Context menus are not
    /// (C4Object.cpp:1961-1980; C4ObjectMenu.cpp:328-435).
    pub(crate) fn open_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        permanent: bool,
        location: Option<Vector2>,
    ) -> Result<(), EngineError> {
        self.build_context_menu(crew_index, base_index, permanent, false, 0)?;
        // C4Command::Context applies Free alignment and SetLocation only
        // after ActivateMenu returns, and only if a menu survived.
        if let Some(location) = location {
            if let Some(menu) = self
                .objects
                .get_mut(crew_index)
                .and_then(|object| object.state.menu.as_mut())
            {
                menu.location = Some(location);
            }
        }
        Ok(())
    }

    fn refill_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        observed_contents_count: i32,
    ) -> Result<(), EngineError> {
        let permanent = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .is_some_and(|menu| menu.permanent);
        self.build_context_menu(
            crew_index,
            base_index,
            permanent,
            true,
            observed_contents_count,
        )
    }

    fn build_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        permanent: bool,
        continue_existing: bool,
        observed_contents_count: i32,
    ) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let crew_id = self.objects[crew_index].id;
        let crew_owner = self.objects[crew_index].state.owner;
        let crew_contents = self.objects[crew_index].state.contents.clone();
        let present_crew_contents = crew_contents
            .into_iter()
            .filter(|object_id| {
                self.find_object_index(*object_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            })
            .collect::<Vec<_>>();
        let first_carried_object = present_crew_contents.first().copied();
        let first_carried_definition = present_crew_contents
            .first()
            .and_then(|object_id| self.find_object_index(*object_id))
            .map(|index| self.objects[index].definition_id.clone());
        let base = &self.objects[base_index];
        let base_id = base.id;
        let base_definition = base.definition_id.clone();
        let base_owner = base.state.owner;
        let base_player = base.state.base;
        let base_is_container = base.state.ocf & ocf::CONTAINER != 0;
        let base_grab_put_get = self.definition_grab_put_get(&base_definition);
        let caption = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());
        let mut items = Vec::new();
        let item = |caption: &str,
                    command: String,
                    item_id: String,
                    symbol: crate::ObjectMenuSymbol| crate::ObjectMenuItem {
            caption: caption.to_string(),
            info_caption: String::new(),
            command,
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id,
            symbol,
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        };

        let continuing_menu = continue_existing
            .then(|| self.objects[crew_index].state.menu.clone())
            .flatten()
            .filter(|menu| {
                menu.identification == Value::Int(14) && menu.refill_object == Some(base_id)
            });
        let (refill_token, previous_token) = if let Some(mut menu) = continuing_menu {
            // DoRefillInternal uses ClearItems(false): the same menu shell and
            // its current Selection remain live while conditions run. The
            // token models the frozen window: row insertion must not select
            // the first item until RefillInternal's final AdjustSelection.
            let previous_token = menu.internal_refill_token;
            let refill_token = next_internal_object_menu_refill_token();
            menu.internal_refill_token = refill_token;
            menu.items.clear();
            self.objects[crew_index].state.menu = Some(menu);
            (refill_token, previous_token)
        } else {
            if continue_existing {
                return Ok(());
            }
            // ActivateMenu closes and initializes C4MN_Context before Refill
            // evaluates any scripted conditions. Keep the live menu installed
            // throughout the build so GetMenu/AddMenuItem/SelectMenuItem
            // observe the same partially populated menu as C++.
            let _ = self.close_object_menu(crew_id, true)?;
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            let refill_token = next_internal_object_menu_refill_token();
            self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
                caption,
                symbol_id: base_definition.clone(),
                title_symbol: crate::ObjectMenuSymbol::default(),
                identification: Value::Int(14),
                style: 1,
                equal_item_height: false,
                permanent,
                location: None,
                runtime_id: next_internal_object_menu_refill_token(),
                extra: crate::ObjectMenuExtra::default(),
                extra_data: 0,
                internal_refill_token: refill_token,
                selection: -1,
                user_menu: false,
                command_object: Some(crew_id),
                scenario_callbacks: false,
                refill_object: Some(base_id),
                refill_object_contents_count: 0,
                location_reset_generation: 0,
                items: Vec::new(),
                columns: 1,
                lines: 0,
                text_progressing: false,
                decoration: None,
            });
            (refill_token, 0)
        };

        let crew_in_base = self.objects[crew_index].state.container == Some(base_id);
        let crew_pushing_base = self.object_procedure(crew_index) == ActionProcedure::Push
            && self.objects[crew_index].state.action.target == Some(base_id);
        if base_is_container
            && (crew_in_base
                || (crew_pushing_base && base_grab_put_get & crate::GRAB_PUT_GET_PUT != 0))
        {
            if let Some(first_carried_definition) = first_carried_definition {
                let command2 = if present_crew_contents.len() > 1
                    || self.selected_crew(crew_owner).len() > 1
                {
                    format!(
                        "PlayerObjectCommand({}, \"Put\", Object({}), 1000, 0) && ExecuteCommand()",
                        crew_owner,
                        base_id.as_u64()
                    )
                } else {
                    String::new()
                };
                items.push(crate::ObjectMenuItem {
                    caption: "Put".to_string(),
                    info_caption: String::new(),
                    command: format!(
                        "PlayerObjectCommand({}, \"Put\", Object({}), 0, 0) && ExecuteCommand()",
                        crew_owner,
                        base_id.as_u64()
                    ),
                    command2,
                    count: C4MN_ITEM_NO_COUNT,
                    item_id: first_carried_definition,
                    symbol: crate::ObjectMenuSymbol::Put,
                    image: crate::ObjectMenuImage::default(),
                    presentation_definition_id: None,
                    picture_snapshot: first_carried_object
                        .and_then(|object| self.native_object_menu_picture_snapshot(object)),
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: None,
                    text_display_progress: -1,
                });
            }
        }
        if base_is_container
            && (crew_in_base
                || (crew_pushing_base && base_grab_put_get & crate::GRAB_PUT_GET_GET != 0)
                || (self.players.contains_key(&base_owner)
                    && !self.players_hostile(base_owner, crew_owner)))
        {
            let mut contents_item = item(
                "Contents",
                format!(
                    "SetCommand(this,\"Get\",Object({}),0,0,,2)&&ExecuteCommand()",
                    base_id.as_u64()
                ),
                base_definition.clone(),
                crate::ObjectMenuSymbol::Definition,
            );
            contents_item.picture_snapshot = self.native_object_menu_picture_snapshot(base_id);
            items.push(contents_item);
        }
        if self.players.contains_key(&base_player) && !self.players_hostile(base_player, crew_owner)
        {
            if self.base_buy_enabled {
                items.push(item(
                    "Buy",
                    format!(
                        "SetCommand(this,\"Buy\",Object({}))&&ExecuteCommand()",
                        base_id.as_u64()
                    ),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Buy { owner: base_player },
                ));
            }
            if self.base_sell_enabled {
                items.push(item(
                    "Sell",
                    format!(
                        "SetCommand(this,\"Sell\",Object({}))&&ExecuteCommand()",
                        base_id.as_u64()
                    ),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Sell { owner: base_player },
                ));
            }
        }
        // AddContextFunctions(target) inserts every native context-function
        // class before BuildInfo/Info/Exit (C4ObjectMenu.cpp:398-408,
        // 544-685).
        for item in items.drain(..) {
            self.add_native_context_menu_item(crew_id, item);
        }
        let _ = self.context_function_menu_items(base_index, crew_id, true)?;
        // AddContextFunctions' final branch exposes the menu Clonk's own
        // context actions when it is inside, pushing, or carrying the clicked
        // target. Building/grab contexts collapse more than two actions into a
        // Clonk submenu; inventory contexts inline every action
        // (C4ObjectMenu.cpp:687-713).
        let crew_container = self.objects[crew_index].state.container;
        let crew_action_target = self.objects[crew_index].state.action.target;
        let crew_is_alive = self.objects[crew_index].state.category & crate::CATEGORY_LIVING == 0
            || self.objects[crew_index].state.alive;
        let base_container = self.objects[base_index].state.container;
        let crew_related_to_target = crew_container == Some(base_id)
            || (self.object_procedure(crew_index) == ActionProcedure::Push
                && crew_action_target == Some(base_id))
            || base_container == Some(crew_id);
        if crew_id != base_id && crew_related_to_target && crew_is_alive {
            let submenu_threshold = (base_container != Some(crew_id)).then_some(2_usize);
            let crew_context_count = self
                .context_function_menu_items(crew_index, crew_id, false)?
                .len();
            if submenu_threshold.is_none_or(|threshold| crew_context_count <= threshold) {
                let _ = self.context_function_menu_items(crew_index, crew_id, true)?;
            } else {
                let crew_definition = self.objects[crew_index].definition_id.clone();
                let crew_name = self
                    .definitions
                    .get(&crew_definition)
                    .map(|definition| definition.name().to_string())
                    .unwrap_or_else(|| crew_definition.clone());
                let mut submenu = item(
                    &crew_name,
                    "SetCommand(this,\"Context\",,0,0,this)&&ExecuteCommand()".to_string(),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Definition,
                );
                submenu.presentation_definition_id = Some(crew_definition);
                submenu.info_caption =
                    "Opens a sub menu with command options for this clonk.".to_string();
                items.push(submenu);
            }
        }
        if self.objects[base_index].state.ocf & ocf::CONSTRUCT != 0
            && self.objects[crew_index].state.rotation == 0
            && self.construction_needs_material
        {
            items.push(item(
                "Construction material",
                format!(
                    "PlayerMessage(GetOwner(), Object({})->GetNeededMatStr(), Object({}))",
                    base_id.as_u64(),
                    base_id.as_u64()
                ),
                "NONE".to_string(),
                crate::ObjectMenuSymbol::Construction,
            ));
        }
        if self
            .definitions
            .get(&base_definition)
            .and_then(|definition| definition.description())
            .is_some()
        {
            items.push(item(
                "Info",
                format!("ShowInfo(Object({}))", base_id.as_u64()),
                base_definition.clone(),
                crate::ObjectMenuSymbol::Info,
            ));
        }
        if base_is_container && self.objects[crew_index].state.container == Some(base_id) {
            items.push(item(
                "Exit",
                "PlayerObjectCommand(GetOwner(),\"Exit\")&&ExecuteCommand()".to_string(),
                "NONE".to_string(),
                crate::ObjectMenuSymbol::Exit,
            ));
        }
        for item in items {
            self.add_native_context_menu_item(crew_id, item);
        }
        if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
            menu.identification == Value::Int(14) && menu.internal_refill_token == refill_token
        }) {
            // RefillInternal's final AdjustSelection uses the selection
            // left by any refill-time callback, not the pre-refill value.
            menu.selection =
                internal_refilled_object_menu_selection(&menu.items, Some(menu.selection), None);
            if continue_existing && menu.refill_object == Some(base_id) {
                menu.refill_object_contents_count = observed_contents_count;
            }
            menu.internal_refill_token = previous_token;
        }
        Ok(())
    }

    /// ShowInfo -> C4Object::ActivateMenu(C4MN_Info): a permanent
    /// information-style menu with the target picture/name and info text
    /// (C4Script.cpp:3332-3336; C4Object.cpp:2008-2027).
    pub(crate) fn open_object_info_menu(
        &mut self,
        crew_index: usize,
        target_index: usize,
    ) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let crew_id = self.objects[crew_index].id;
        let target_id = self.objects[target_index].id;
        // C4Object::ActivateMenu closes and initializes the new Info menu
        // before evaluating GetInfoString while adding its first item.
        let _ = self.close_object_menu(crew_id, false)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(());
        };
        let definition_id = self.objects[target_index].definition_id.clone();
        let state = self.objects[target_index].script_state_snapshot();
        let (name, mut info_caption, action_library) = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            let name = state
                .custom_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| definition.name().to_string());
            (
                name,
                definition.description().unwrap_or_default().to_string(),
                definition.action_library().clone(),
            )
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: name.clone(),
            symbol_id: "NONE".to_string(),
            title_symbol: crate::ObjectMenuSymbol::InfoTitle,
            identification: Value::Int(15),
            style: 2,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            internal_refill_token: 0,
            selection: -1,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: None,
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items: Vec::new(),
            columns: 1,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });

        let effect_call = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            definition.call_object_effect_info(
                &state,
                target_id,
                self.rng.clone(),
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.frame,
                self.host_world_context(),
                self.game_over_triggered,
                self.audio_registry.clone(),
            )
        }?;
        let (effect_lines, outcome, audio, rng) = effect_call;
        self.rng = rng;
        self.audio_registry = audio;
        self.apply_action_callback_outcome(
            target_index,
            outcome,
            &action_library,
            target_id,
            &definition_id,
        )?;
        for line in effect_lines {
            if !info_caption.is_empty() {
                info_caption.push('|');
            }
            info_caption.push_str(&line);
        }
        let item = crate::ObjectMenuItem {
            caption: name.clone(),
            info_caption: crate::normalize_menu_info_caption(info_caption),
            command: String::new(),
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id: definition_id.clone(),
            symbol: crate::ObjectMenuSymbol::default(),
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: Some(target_id),
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        };
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        if let Some(menu) = self.objects[crew_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.identification == Value::Int(15))
        {
            menu.items.push(item);
            menu.selection = 0;
        }
        Ok(())
    }

    /// `C4Player::DirectCom` (C4Player.cpp:1453-1488): the cursor coms'
    /// script-override half (`Cursor->CallControl`, :1457-1475) and the
    /// crew-cycling dispatch (:1479-1485); everything else goes to the
    /// cursor object via ObjectCom.
    pub fn player_direct_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        let plain_cursor = matches!(
            com & !COM_DOUBLE,
            COM_CURSOR_LEFT | COM_CURSOR_RIGHT | COM_CURSOR_TOGGLE
        );
        if plain_cursor {
            // Cursor object override (:1457-1475).
            if !self.is_owner_eliminated(owner) {
                if let Some(index) = self
                    .crew_cursor(owner)
                    .and_then(|cursor| self.find_object_index(cursor))
                {
                    self.objects[index].state.controller = owner;
                    if self.object_call_control(index, owner, com, None)? {
                        if com & COM_DOUBLE == 0 {
                            self.player_update_selection_toggle_status(owner)?;
                        }
                        return Ok(());
                    }
                }
            }
            // Crew cycling (:1479-1485).
            match com & !COM_DOUBLE {
                COM_CURSOR_LEFT => self.player_cursor_left(owner)?,
                COM_CURSOR_RIGHT => self.player_cursor_right(owner)?,
                COM_CURSOR_TOGGLE => {
                    if com & COM_DOUBLE != 0 {
                        self.player_select_all_crew(owner)?;
                    } else {
                        self.player_cursor_toggle(owner)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        // Everything else routes to the cursor object (C4Player.cpp:1486);
        // menu-com leftovers get swallowed in object_direct_com like
        // C4Object.cpp:3356-3357 (object menus live in the app layer).
        self.player_object_com(owner, com, data)
    }

    /// `C4Player::ObjectCom` (C4Player.cpp:1367-1390): commit the cursor
    /// selection on regular coms, then route the com to the cursor object
    /// with an updated controller.
    fn player_object_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        // Eliminated (:1369).
        if self.is_owner_eliminated(owner) {
            return Ok(());
        }
        // ObjectCom hides the startup control hint before selection commits
        // or any cursor callback can reflect the player (C4Player.cpp:
        // 1368-1379). Cursor-cycle DirectCom paths above deliberately bypass
        // this write.
        if let Some(player) = self.players.get_mut(&owner) {
            player.hide_startup();
        }
        // If regular com, update cursor & selection status (:1378-1379).
        let is_release = (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com);
        if com & (COM_SINGLE | COM_DOUBLE) == 0 && !is_release {
            self.player_update_selection_toggle_status(owner)?;
        }
        self.ensure_cursor(owner)?;
        let Some(cursor) = self.crew_cursor(owner) else {
            return Ok(());
        };
        let Some(index) = self.find_object_index(cursor) else {
            return Ok(());
        };
        self.objects[index].state.controller = owner;
        self.object_direct_com(index, com, data)
    }

    // ---- Cursor selection model (C4Player.cpp:1235-1365) ------------------

    /// `C4Object::DoSelect` (C4Object.cpp:5815-5824): CrewDisabled guard,
    /// the Select flag unless cursor-only, and the `~CrewSelection(false,
    /// fCursor)` callback.
    pub(super) fn object_do_select(
        &mut self,
        index: usize,
        _owner: i32,
        cursor_only: bool,
    ) -> Result<(), EngineError> {
        if self.objects[index].state.crew_disabled {
            return Ok(());
        }
        if !cursor_only {
            self.objects[index].state.selected = true;
        }
        self.contained_call(
            index,
            "CrewSelection",
            &[Value::Bool(false), Value::Bool(cursor_only)],
        )?;
        Ok(())
    }

    /// `C4Object::UnSelect` (C4Object.cpp:5826-5832).
    pub(super) fn object_un_select(
        &mut self,
        index: usize,
        _owner: i32,
        cursor_only: bool,
    ) -> Result<(), EngineError> {
        if !cursor_only {
            self.objects[index].state.selected = false;
        }
        self.contained_call(
            index,
            "CrewSelection",
            &[Value::Bool(true), Value::Bool(cursor_only)],
        )?;
        Ok(())
    }

    /// `C4Player::SetCursor` (C4Player.cpp:1831-1847).
    pub(super) fn player_set_cursor(
        &mut self,
        owner: i32,
        target: Option<ObjectId>,
        select_flash: bool,
        select_arrow: bool,
    ) -> Result<(), EngineError> {
        // Check disabled (:1834).
        let target_index = target.and_then(|target| self.find_object_index(target));
        if target.is_some() && target_index.is_none() {
            return Ok(());
        }
        if target_index.is_some_and(|index| self.objects[index].state.crew_disabled) {
            return Ok(());
        }
        let previous = self.crew_cursor(owner);
        let changed = previous != target;
        if let Some(target) = target {
            self.crew_selection
                .entry(owner)
                .or_default()
                .set_cursor(Some(target));
        } else {
            self.crew_selection.remove(&owner);
        }
        // Cursor is assigned before either callback (C4Player.cpp:1838), so
        // callback-side GetCursor observes the new object/null immediately.
        if let Some(player) = self.players.get_mut(&owner) {
            player.set_cursor(target);
        }
        // Unselect previous (:1841).
        if let Some(previous_index) = previous
            .filter(|_| changed)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(previous_index, owner, true)?;
        }
        // Select object (:1843).
        if let Some(target_index) = target_index {
            self.object_do_select(target_index, owner, true)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            if select_arrow {
                player.control.cursor_flash = 30;
            }
            if select_flash {
                player.control.select_flash = 30;
            }
        }
        Ok(())
    }

    /// The player's raw C4Player::Crew order. Inactive objects remain linked;
    /// only a deleted pointer disappears through ClearPointers.
    fn player_crew_roster(&self, owner: i32) -> Vec<ObjectId> {
        self.players
            .get(&owner)
            .map(|player| player.crew().to_vec())
            .unwrap_or_else(|| self.crew_members(owner))
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status != crate::ObjectStatus::Deleted
                })
            })
            .collect()
    }

    /// `C4Player::GetHiRankActiveCrew` (C4Player.cpp:1003-1021): the
    /// strictly highest-ranked eligible member wins. Equal ranks retain the
    /// FIRST eligible roster entry because the C++ loop only replaces its
    /// candidate for `iRank > iHighestRank`.
    pub(super) fn player_hi_rank_active_crew(
        &self,
        owner: i32,
        select_only: bool,
    ) -> Option<ObjectId> {
        let selected = self.selected_crew(owner);
        let mut highest: Option<(ObjectId, i32)> = None;
        for id in self.player_crew_roster(owner) {
            let eligible = self
                .find_object_index(id)
                .is_some_and(|index| !self.objects[index].state.crew_disabled)
                && (!select_only || selected.contains(&id));
            if !eligible {
                continue;
            }
            let rank = self.crew_ranks.get(&id.as_u64()).copied().unwrap_or(-1);
            match highest {
                Some((_, highest_rank)) if highest_rank >= rank => {}
                _ => highest = Some((id, rank)),
            }
        }
        highest.map(|(id, _)| id)
    }

    /// `C4Player::AdjustCursorCommand` (C4Player.cpp:1235-1258).
    pub(super) fn player_adjust_cursor_command(&mut self, owner: i32) -> Result<(), EngineError> {
        // ResetCursorView runs before the replacement search while the old
        // ViewCursor is still live (C4Player.cpp:1241-1244).
        if let Some(player) = self.players.get_mut(&owner) {
            player.reset_cursor_view();
        }
        // Find hirank Select, else any (:1240-1245).
        let hi_rank = self
            .player_hi_rank_active_crew(owner, true)
            .or_else(|| self.player_hi_rank_active_crew(owner, false));
        let previous = self
            .players
            .get(&owner)
            .and_then(|player| player.cursor())
            .or_else(|| self.crew_cursor(owner));
        if previous != hi_rank {
            self.crew_selection
                .entry(owner)
                .or_default()
                .set_cursor(hi_rank);
            if let Some(player) = self.players.get_mut(&owner) {
                player.set_cursor(hi_rank);
            }
            // UpdateView precedes both selection callbacks and resolves the
            // still-live ViewCursor before ClearPointers' suffix clears it.
            self.update_player_view(owner);
        }
        // UnSelect previous cursor (:1253).
        if let Some(previous_index) = previous
            .filter(|id| Some(*id) != hi_rank)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(previous_index, owner, true)?;
        }
        // We have a cursor: do select it (:1255) — the non-cursor DoSelect
        // sets the Select flag too.
        let live_cursor = self
            .players
            .get(&owner)
            .and_then(|player| player.cursor())
            .or_else(|| self.crew_cursor(owner));
        if let Some(cursor_index) = live_cursor.and_then(|id| self.find_object_index(id)) {
            self.object_do_select(cursor_index, owner, false)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_flash = 30;
        }
        Ok(())
    }

    /// `C4Player::CursorRight` (C4Player.cpp:1261-1275).
    fn player_cursor_right(&mut self, owner: i32) -> Result<(), EngineError> {
        self.player_cursor_step(owner, false)
    }

    /// `C4Player::CursorLeft` (C4Player.cpp:1278-1293).
    fn player_cursor_left(&mut self, owner: i32) -> Result<(), EngineError> {
        self.player_cursor_step(owner, true)
    }

    fn player_cursor_step(&mut self, owner: i32, backwards: bool) -> Result<(), EngineError> {
        let mut roster = self.player_crew_roster(owner);
        if backwards {
            roster.reverse();
        }
        let eligible = |engine: &Self, id: ObjectId| {
            engine
                .find_object_index(id)
                .is_some_and(|index| !engine.objects[index].state.crew_disabled)
        };
        // Walk on from the cursor's link; falling off the end rescans the
        // whole list from the front (C4Player.cpp:1264-1270).
        let next = self
            .crew_cursor(owner)
            .and_then(|cursor| roster.iter().position(|id| *id == cursor))
            .and_then(|position| {
                roster[position + 1..]
                    .iter()
                    .copied()
                    .find(|id| eligible(self, *id))
            })
            .or_else(|| roster.iter().copied().find(|id| eligible(self, *id)));
        if let Some(target) = next {
            self.player_set_cursor(owner, Some(target), false, true)?;
        }
        // Updates (:1272-1274).
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_flash = 30;
            player.control.cursor_selection = 1;
        }
        Ok(())
    }

    /// The object number queued by `C4MouseControl::SendPlayerSelectNext`:
    /// advance in crew-list order, wrapping to the first eligible member
    /// (C4MouseControl.cpp:1284-1300).
    pub fn player_mouse_select_next_object(&self, owner: i32) -> Option<ObjectId> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        let roster = self.player_crew_roster(owner);
        let eligible = |engine: &Self, id: ObjectId| {
            engine.find_object_index(id).is_some_and(|index| {
                !engine.objects[index].destroyed
                    && engine.objects[index].state.status != crate::ObjectStatus::Deleted
                    && !engine.objects[index].state.crew_disabled
            })
        };
        let next = self
            .crew_cursor(owner)
            .and_then(|cursor| roster.iter().position(|id| *id == cursor))
            .and_then(|position| {
                roster[position + 1..]
                    .iter()
                    .copied()
                    .find(|id| eligible(self, *id))
            })
            .or_else(|| roster.iter().copied().find(|id| eligible(self, *id)));
        next
    }

    /// Select-next followed by the synchronized one-object packet execution.
    pub fn player_mouse_select_next(&mut self, owner: i32) -> Result<bool, EngineError> {
        let Some(next) = self.player_mouse_select_next_object(owner) else {
            return Ok(false);
        };
        self.execute_player_select(&PlayerSelectControlData {
            player: owner,
            objects: vec![next.as_u64() as i32],
            by_client: -1,
        })
    }

    /// Crew objects inside C4MouseControl's landscape drag frame, in the
    /// player's stored crew-list order. The mouse oracle compares object
    /// origins (not shape rectangles), includes both frame edges, and skips
    /// CrewDisabled entries (C4MouseControl.cpp:610-624).
    pub fn mouse_drag_crew_in_rect(
        &self,
        owner: i32,
        first: Vector2,
        second: Vector2,
    ) -> Vec<ObjectId> {
        let min_x = first.x.min(second.x);
        let max_x = first.x.max(second.x);
        let min_y = first.y.min(second.y);
        let max_y = first.y.max(second.y);
        self.player_crew_roster(owner)
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.state.crew_disabled
                        && (min_x..=max_x).contains(&object.state.position.x)
                        && (min_y..=max_y).contains(&object.state.position.y)
                })
            })
            .collect()
    }

    /// Carryable objects inside C4MouseControl's landscape drag frame, in the
    /// C++ `Game.Objects` main-list order and capped at 20. Object origins are
    /// compared against inclusive frame edges; contained objects never enter
    /// this local mouse selection (C4MouseControl.cpp:626-645).
    pub fn mouse_drag_carryables_in_rect(&self, first: Vector2, second: Vector2) -> Vec<ObjectId> {
        let min_x = first.x.min(second.x);
        let max_x = first.x.max(second.x);
        let min_y = first.y.min(second.y);
        let max_y = first.y.max(second.y);
        // `exec_list` is the reverse of C++'s main list (lib.rs:11372-11380).
        self.exec_list
            .iter()
            .rev()
            .filter_map(|id| {
                self.find_object_index(*id)
                    .map(|index| &self.objects[index])
            })
            .filter(|object| {
                object.state.status.is_active()
                    && object.state.ocf & ocf::CARRYABLE != 0
                    && object.state.container.is_none()
                    && (min_x..=max_x).contains(&object.state.position.x)
                    && (min_y..=max_y).contains(&object.state.position.y)
            })
            .map(|object| object.id)
            .take(20)
            .collect()
    }

    /// The carryable-object cursor selected by `C4MouseControl::DragMoving`
    /// at a world pixel. Control-modified Put is deliberately left to the
    /// separate region/container slice; ordinary liquid/near-ground Drop and
    /// ballistic Throw are exact (C4MouseControl.cpp:833-879;
    /// C4Landscape.cpp:2055-2100).
    pub fn mouse_drag_carryable_cursor(
        &mut self,
        owner: i32,
        position: Vector2,
    ) -> Option<MouseDragCarryableCursor> {
        {
            let landscape = self.landscape.as_ref()?;
            if landscape.is_liquid_at(position.x, position.y) {
                return Some(MouseDragCarryableCursor::Drop);
            }
            if landscape.is_solid_at(position.x, position.y) {
                return None;
            }

            let mut ground_y = position.y;
            let landscape_height = landscape.estimated_height();
            while ground_y < landscape_height && !landscape.is_solid_at(position.x, ground_y) {
                ground_y += 1;
            }
            if (ground_y - position.y).abs() <= 5 {
                return Some(MouseDragCarryableCursor::Drop);
            }
        }

        let throw_force = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
            .map(|index| math::val_by_physical(400, self.object_physical(index).throw))
            .unwrap_or_else(|| math::val_by_physical(400, 50_000));
        // GetPhysical may fill a fair-crew projection and execute definition
        // script. Native re-reads pPlayer->Cursor separately for direction
        // and throwing height after that call (C4MouseControl.cpp:866-870).
        let (cursor_x, throw_height) = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
            .map(|index| {
                (
                    self.objects[index].state.position.x,
                    self.objects[index]
                        .current_shape_rect()
                        .map(|rect| rect.height)
                        .unwrap_or(20),
                )
            })
            .unwrap_or((position.x, 20));
        let preferred_direction = if cursor_x > position.x { -1 } else { 1 };
        let landscape = self.landscape.as_ref()?;
        [preferred_direction, -preferred_direction]
            .into_iter()
            .find_map(|direction| {
                landscape
                    .find_throwing_position(
                        position,
                        FixedVec2::new(throw_force * direction, -throw_force),
                        throw_height,
                        self.physics.gravity_as_c4fixed(),
                    )
                    .map(|landing| MouseDragCarryableCursor::Throw { direction, landing })
            })
    }

    pub fn mouse_drag_carryable_command(
        &mut self,
        owner: i32,
        position: Vector2,
    ) -> Option<CommandId> {
        self.mouse_drag_carryable_cursor(owner, position)
            .map(|cursor| match cursor {
                MouseDragCarryableCursor::Drop => CommandId::Drop,
                MouseDragCarryableCursor::Throw { .. } => CommandId::Throw,
            })
    }

    fn mouse_world_point_is_solid(&self, point: Vector2) -> bool {
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        landscape.is_solid_at(point.x, point.y)
            || self
                .ocf_solid_mask_overlay()
                .iter()
                .any(|mask| mask.contains(point.x, point.y))
    }

    /// Reproduce `C4MouseControl::UpdateCursorTarget`'s world-cursor
    /// priority. A picked object first replaces the landscape cursor with a
    /// crosshair; each later matching object cursor then overrides the
    /// previous one, and the nearby jump cursor is evaluated last
    /// (C4MouseControl.cpp:451-538).
    pub fn mouse_world_cursor(
        &self,
        owner: i32,
        target: Option<ObjectId>,
        point: Vector2,
        control_down: bool,
    ) -> MouseWorldCursor {
        let mut cursor = if self.mouse_world_point_is_solid(point) {
            MouseWorldCursor::Dig {
                material: control_down,
            }
        } else {
            MouseWorldCursor::Crosshair
        };

        if let Some(target) = target {
            let Some(index) = self.find_object_index(target) else {
                return cursor;
            };
            let object = &self.objects[index];
            if !object.state.status.is_active() || object.state.container.is_some() {
                return cursor;
            }

            // Any object admitted by the primary OCF pick suppresses the
            // landscape dig cursor, even if it was admitted by OCF_Exclusive
            // and has no actionable cursor of its own.
            cursor = MouseWorldCursor::Crosshair;
            let target_ocf = self.object_ocf_for_pos(index, point);

            // The first entrance check intentionally uses cached Entrance:
            // containers remain enterable across their whole shape. The
            // ordinary position-filtered Entrance check below runs later.
            if target_ocf & ocf::CONTAINER != 0 && object.state.ocf & ocf::ENTRANCE != 0 {
                cursor = MouseWorldCursor::Enter(target);
            }
            if target_ocf & ocf::GRAB != 0 {
                let pushing_target = self
                    .crew_cursor(owner)
                    .and_then(|cursor| self.find_object_index(cursor))
                    .is_some_and(|cursor_index| {
                        self.object_procedure(cursor_index) == ActionProcedure::Push
                            && self.objects[cursor_index].state.action.target == Some(target)
                    });
                cursor = if pushing_target {
                    MouseWorldCursor::Ungrab(target)
                } else {
                    MouseWorldCursor::Grab(target)
                };
            }
            if target_ocf & ocf::CARRYABLE != 0 {
                cursor = if target_ocf & ocf::IN_SOLID != 0 {
                    MouseWorldCursor::DigObject(target)
                } else {
                    MouseWorldCursor::Carryable(target)
                };
            }
            if target_ocf & ocf::CHOP != 0 {
                let width = object
                    .current_shape_rect()
                    .map(|shape| shape.width)
                    .unwrap_or(0);
                let dx = point.x - object.state.position.x;
                let dy = point.y - object.state.position.y;
                if (-width / 3..=width / 3).contains(&dx) && (-width / 2..=width / 3).contains(&dy)
                {
                    cursor = MouseWorldCursor::Chop(target);
                }
            }
            if target_ocf & ocf::ENTRANCE != 0 {
                cursor = MouseWorldCursor::Enter(target);
            }
            if target_ocf & ocf::CONSTRUCT != 0 {
                cursor = MouseWorldCursor::Build(target);
            }
            if target_ocf & ocf::ALIVE != 0 && self.player_crew_roster(owner).contains(&target) {
                cursor = MouseWorldCursor::Select(target);
            }
            if object.state.category & CATEGORY_MOUSE_SELECT != 0 {
                cursor = MouseWorldCursor::Select(target);
            }
            if object.state.ocf & ocf::ALIVE != 0
                && object.state.alive
                && self.players_hostile(owner, object.state.owner)
            {
                cursor = MouseWorldCursor::Attack(target);
            }
        }

        self.mouse_jump_cursor(owner, point).unwrap_or(cursor)
    }

    /// Build the exact `C4ControlPlayerCommand` produced by a world
    /// LeftDouble event. Selection and Jump cursors deliberately do not emit
    /// a double-click command (C4MouseControl.cpp:982-1007).
    pub fn mouse_left_double_command(
        &self,
        owner: i32,
        target: Option<ObjectId>,
        point: Vector2,
        control_down: bool,
        shift_down: bool,
    ) -> Option<PlayerCommandControlData> {
        if !self.players.contains_key(&owner) || self.is_owner_eliminated(owner) {
            return None;
        }

        let (action, target) = match self.mouse_world_cursor(owner, target, point, control_down) {
            MouseWorldCursor::Attack(target) => (MouseDoubleClickAction::Attack, Some(target)),
            MouseWorldCursor::Grab(target) => (MouseDoubleClickAction::Grab, Some(target)),
            MouseWorldCursor::Ungrab(target) => (MouseDoubleClickAction::Ungrab, Some(target)),
            MouseWorldCursor::Build(target) => (MouseDoubleClickAction::Build, Some(target)),
            MouseWorldCursor::Chop(target) => (MouseDoubleClickAction::Chop, Some(target)),
            MouseWorldCursor::Enter(target) => (MouseDoubleClickAction::Enter, Some(target)),
            MouseWorldCursor::Carryable(target) | MouseWorldCursor::DigObject(target) => {
                (MouseDoubleClickAction::Get, Some(target))
            }
            MouseWorldCursor::Dig { material } => (MouseDoubleClickAction::Dig { material }, None),
            MouseWorldCursor::Crosshair
            | MouseWorldCursor::Select(_)
            | MouseWorldCursor::JumpLeft
            | MouseWorldCursor::JumpRight => return None,
        };

        self.mouse_left_double_command_for_action(owner, action, target, point, shift_down)
    }

    /// Build LeftDouble from the cursor and target identities retained by the
    /// last Move/Tick5 refill (C4MouseControl.cpp:982-1007).
    pub fn mouse_left_double_command_for_action(
        &self,
        owner: i32,
        action: MouseDoubleClickAction,
        target: Option<ObjectId>,
        point: Vector2,
        shift_down: bool,
    ) -> Option<PlayerCommandControlData> {
        if !self.players.contains_key(&owner) || self.is_owner_eliminated(owner) {
            return None;
        }

        let target = target.map_or(0, |target| target.as_u64() as i32);
        let (command, x, y, target, data) = match action {
            MouseDoubleClickAction::Attack => (CommandId::Attack, point.x, point.y, target, 0),
            MouseDoubleClickAction::Grab => (CommandId::Grab, 0, 0, target, 0),
            MouseDoubleClickAction::Ungrab => (CommandId::UnGrab, point.x, point.y, target, 0),
            MouseDoubleClickAction::Build => (CommandId::Build, point.x, point.y, target, 0),
            MouseDoubleClickAction::Chop => (CommandId::Chop, point.x, point.y, target, 0),
            MouseDoubleClickAction::Enter => (CommandId::Enter, point.x, point.y, target, 0),
            MouseDoubleClickAction::Get => (CommandId::Get, 0, 0, target, 0),
            MouseDoubleClickAction::Dig { material } => {
                (CommandId::Dig, point.x, point.y, 0, i32::from(material))
            }
        };

        Some(PlayerCommandControlData {
            player: owner,
            command: command as i32,
            x,
            y,
            target,
            target2: 0,
            data,
            add_mode: C4P_COMMAND_SET | if shift_down { C4P_COMMAND_APPEND } else { 0 },
            by_client: -1,
        })
    }

    /// Whether `point` selects the nearby jump cursor for the player's cursor
    /// object. UpdateCursorTarget evaluates this after every object cursor, so
    /// it also overrides Select (C4MouseControl.cpp:522-534).
    fn mouse_jump_cursor(&self, owner: i32, point: Vector2) -> Option<MouseWorldCursor> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        self.crew_cursor(owner)
            .and_then(|cursor| self.find_object_index(cursor))
            .and_then(|cursor_index| {
                let cursor = &self.objects[cursor_index];
                if cursor.state.container.is_some()
                    || self.object_procedure(cursor_index) != ActionProcedure::Walk
                {
                    return None;
                }
                let dx = point.x - cursor.state.position.x;
                let dy = point.y - cursor.state.position.y;
                if !(-25..=-10).contains(&dy) {
                    return None;
                }
                if (-15..=-1).contains(&dx) {
                    Some(MouseWorldCursor::JumpLeft)
                } else if (1..=15).contains(&dx) {
                    Some(MouseWorldCursor::JumpRight)
                } else {
                    None
                }
            })
    }

    pub fn mouse_jump_zone(&self, owner: i32, point: Vector2) -> bool {
        self.mouse_jump_cursor(owner, point).is_some()
    }

    /// Classify the down cursor which may start a world-object moving drag.
    /// This follows UpdateCursorTarget's OCF priority through the later
    /// Chop/Enter/Build/Select/Attack/Jump overrides, then DragNone's strict
    /// `Def->Grab == 1` vehicle gate (C4MouseControl.cpp:474-538,922-941).
    pub fn mouse_world_drag_source(
        &self,
        owner: i32,
        target: ObjectId,
        point: Vector2,
    ) -> Option<MouseDragSource> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        let index = self.find_object_index(target)?;
        let object = &self.objects[index];
        if !object.state.status.is_active() || object.state.container.is_some() {
            return None;
        }
        let grab = self.definitions.get(&object.definition_id)?.grab();
        match self.mouse_world_cursor(owner, Some(target), point, false) {
            MouseWorldCursor::Carryable(_) | MouseWorldCursor::DigObject(_) => {
                Some(MouseDragSource::Carryable)
            }
            MouseWorldCursor::Grab(_) | MouseWorldCursor::Ungrab(_) if grab == 1 => {
                Some(MouseDragSource::Vehicle)
            }
            _ => None,
        }
    }

    /// The moving-drag class for a copied viewport region target. Regions
    /// use cached OCF_Carryable but the definition's raw Grab=1 value rather
    /// than the world cursor's position-filtered OCF (C4MouseControl.cpp:
    /// 942-961).
    pub fn mouse_region_drag_source(&self, target: ObjectId) -> Option<MouseDragSource> {
        let index = self.find_object_index(target)?;
        let object = &self.objects[index];
        if object.state.ocf & ocf::CARRYABLE != 0 {
            return Some(MouseDragSource::Carryable);
        }
        self.definitions
            .get(&object.definition_id)
            .filter(|definition| definition.grab() == 1)
            .map(|_| MouseDragSource::Vehicle)
    }

    /// Build C4MouseControl's local Selection when dragging from a viewport
    /// region. A right drag expands when `Contents.ObjectCount(id)` finds
    /// multiple nonzero-Status matches. The raw link walk then inserts via
    /// `C4ObjectList::Add`, which applies the same Status filter; otherwise
    /// it tries to add only the copied region target
    /// (C4MouseControl.cpp:942-961).
    pub fn mouse_region_drag_objects(&self, target: ObjectId, right_button: bool) -> Vec<ObjectId> {
        if self.mouse_region_drag_source(target).is_none() {
            return Vec::new();
        }
        let Some(index) = self.find_object_index(target) else {
            return Vec::new();
        };
        let object = &self.objects[index];
        let single_target = || {
            object
                .has_nonzero_status()
                .then_some(vec![target])
                .unwrap_or_default()
        };
        let Some(container) = right_button.then_some(object.state.container).flatten() else {
            return single_target();
        };
        let Some(container_index) = self.find_object_index(container) else {
            return single_target();
        };
        let same_id = self.objects[container_index]
            .state
            .contents
            .iter()
            .copied()
            .filter(|candidate| {
                self.find_object_index(*candidate)
                    .is_some_and(|candidate_index| {
                        let candidate = &self.objects[candidate_index];
                        candidate.has_nonzero_status()
                            && candidate.definition_id == object.definition_id
                    })
            })
            .collect::<Vec<_>>();
        if same_id.len() > 1 {
            same_id
        } else {
            single_target()
        }
    }

    /// Execute the crew half of `C4ControlPlayerSelect`: replace the current
    /// selection, adjust the cursor, and arm the selection flash. Requested
    /// ids are rechecked against the live crew roster at execution time
    /// (C4Control.cpp:341-369; C4Player.cpp:1848-1862).
    pub fn player_mouse_select_crew<I>(
        &mut self,
        owner: i32,
        requested: I,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let objects = requested
            .into_iter()
            .filter_map(|id| i32::try_from(id.as_u64()).ok())
            .collect();
        self.execute_player_select(&PlayerSelectControlData {
            player: owner,
            objects,
            by_client: -1,
        })
    }

    /// `C4Player::UnselectCrew` (C4Player.cpp:1295-1306).
    pub(super) fn player_unselect_crew(&mut self, owner: i32) -> Result<(), EngineError> {
        let cursor = self.crew_cursor(owner);
        let mut cursor_deselected = false;
        for id in self.player_crew_roster(owner) {
            if cursor == Some(id) {
                cursor_deselected = true;
            }
            if let Some(index) = self.find_object_index(id) {
                self.object_un_select(index, owner, false)?;
            }
        }
        // A cursor outside the crew unselects too (:1305).
        if let Some(cursor_index) = cursor
            .filter(|_| !cursor_deselected)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(cursor_index, owner, false)?;
        }
        Ok(())
    }

    /// `C4Player::SelectSingleByCursor` (C4Player.cpp:1308-1317).
    fn player_select_single_by_cursor(&mut self, owner: i32) -> Result<(), EngineError> {
        self.player_unselect_crew(owner)?;
        if let Some(cursor_index) = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_do_select(cursor_index, owner, false)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
        }
        self.player_adjust_cursor_command(owner)
    }

    /// `C4Player::CursorToggle` (C4Player.cpp:1319-1339).
    fn player_cursor_toggle(&mut self, owner: i32) -> Result<(), EngineError> {
        let cursor_selection = self
            .players
            .get(&owner)
            .map(|player| player.control.cursor_selection)
            .unwrap_or(0);
        if cursor_selection != 0 {
            // Selection mode: toggle cursor select (:1323-1327).
            if let Some(cursor) = self.crew_cursor(owner) {
                let selected = self
                    .find_object_index(cursor)
                    .is_some_and(|index| self.objects[index].state.selected);
                if let Some(index) = self.find_object_index(cursor) {
                    if selected {
                        self.object_un_select(index, owner, false)?;
                    } else {
                        self.object_do_select(index, owner, false)?;
                    }
                }
            }
            if let Some(player) = self.players.get_mut(&owner) {
                player.control.cursor_toggled = 1;
            }
        } else {
            // Pure toggle: toggle all Select (:1329-1336).
            for id in self.player_crew_roster(owner) {
                let Some(index) = self.find_object_index(id) else {
                    continue;
                };
                if self.objects[index].state.crew_disabled {
                    continue;
                }
                let selected = self.objects[index].state.selected;
                if selected {
                    self.object_un_select(index, owner, false)?;
                } else {
                    self.object_do_select(index, owner, false)?;
                }
            }
            self.player_adjust_cursor_command(owner)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
        }
        Ok(())
    }

    /// `C4Player::SelectAllCrew` (C4Player.cpp:1341-1353).
    fn player_select_all_crew(&mut self, owner: i32) -> Result<(), EngineError> {
        for id in self.player_crew_roster(owner) {
            if let Some(index) = self.find_object_index(id) {
                self.object_do_select(index, owner, false)?;
            }
        }
        self.player_adjust_cursor_command(owner)?;
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
            player.control.select_flash = 30;
        }
        // Game display (:1352): the app is the local player's view.
        self.emit_audio_command(crate::AudioCommand::PlaySound {
            name: "Ding".to_string(),
            target: None,
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
            target_position: None,
        });
        Ok(())
    }

    /// `C4Player::UpdateSelectionToggleStatus` (C4Player.cpp:1355-1365).
    fn player_update_selection_toggle_status(&mut self, owner: i32) -> Result<(), EngineError> {
        let (cursor_selection, cursor_toggled) = self
            .players
            .get(&owner)
            .map(|player| {
                (
                    player.control.cursor_selection,
                    player.control.cursor_toggled,
                )
            })
            .unwrap_or((0, 0));
        if cursor_selection != 0 {
            if cursor_toggled != 0 {
                self.player_adjust_cursor_command(owner)?;
            } else {
                self.player_select_single_by_cursor(owner)?;
            }
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
        Ok(())
    }

    /// `C4Object::DirectCom` (C4Object.cpp:3327-3557).
    pub(crate) fn object_direct_com(
        &mut self,
        index: usize,
        com: u8,
        data: i32,
    ) -> Result<(), EngineError> {
        let is_release = (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com);
        let plain_com = if is_release {
            com - COM_RELEASE_OFFSET
        } else {
            com & !(COM_SINGLE | COM_DOUBLE)
        };
        let is_cursor = (COM_CURSOR_FIRST..=COM_CURSOR_LAST).contains(&plain_com);

        // We only want the script callbacks for cursor controls (:3339-3347).
        if is_cursor {
            let controller = self.objects[index].state.controller;
            if self.players.contains_key(&controller) {
                self.object_call_control(index, controller, com, None)?;
            }
            return Ok(());
        }

        // COM_Special and COM_Contents bypass an active object menu;
        // every other com goes to Menu->Control first, whose active-menu
        // return consumes even unrecognized raw/release coms
        // (C4Object.cpp:3349-3367; C4Menu.cpp:433-480).
        let bypass_menu = plain_com == COM_SPECIAL || com == COM_CONTENTS;
        if !bypass_menu && self.object_menu_control(index, com, data)? {
            return Ok(());
        }

        // Menu com leftovers from a menu closed before execution are
        // swallowed (C4Object.cpp:3369-3371).
        if (COM_MENU_NAVIGATION1..=COM_MENU_NAVIGATION2).contains(&com) {
            return Ok(());
        }

        // Decrease NoCollectDelay (:3359-3362): plain (non-Single/Double,
        // non-release) coms count the drop's collection delay down; the
        // ObjectComDrop arm that sets it lives with the command layer.
        if com & COM_SINGLE == 0 && com & COM_DOUBLE == 0 && !is_release {
            let delay = &mut self.objects[index].state.no_collect_delay;
            if *delay > 0 {
                *delay -= 1;
            }
        }

        // COM_Contents contents shift (:3364-3372): data carries the target
        // object NUMBER (not ID); the shift always runs on the target's
        // container, which is not necessarily this object.
        if com == COM_CONTENTS {
            let target_id = ObjectId::new(data as u64);
            if let Some(container_index) = self
                .find_object_index(target_id)
                .filter(|&target_index| self.objects[target_index].has_nonzero_status())
                .and_then(|target_index| self.objects[target_index].state.container)
                .and_then(|container_id| self.find_object_index(container_id))
            {
                self.object_direct_com_contents(container_index, target_id, true)?;
            }
            return Ok(());
        }

        // Contained control (except specials) (:3374-3379).
        if let Some(container) = self.objects[index].state.container {
            if plain_com != COM_SPECIAL
                && plain_com != COM_SPECIAL2
                && com != COM_WHEEL_UP
                && com != COM_WHEEL_DOWN
            {
                if let Some(container_index) = self.find_object_index(container) {
                    let controller = self.objects[index].state.controller;
                    self.objects[container_index].state.controller = controller;
                    self.object_contained_control(index, com)?;
                }
                return Ok(());
            }
        }

        // Regular DirectCom clears commands (:3381-3383).
        if com & (COM_SINGLE | COM_DOUBLE) == 0 && !is_release {
            self.objects[index].apply_command_operations([CommandOperation::Clear]);
        }

        // Object script override — CallControl runs for EVERY com (:3385-3389).
        let controller = self.objects[index].state.controller;
        let has_controller = self.players.contains_key(&controller);
        if has_controller && self.object_call_control(index, controller, com, None)? {
            return Ok(());
        }

        // Direct wheel control (:3391-3396): scroll contents.
        if com == COM_WHEEL_UP || com == COM_WHEEL_DOWN {
            self.object_shift_contents(index, com == COM_WHEEL_UP, true)?;
            return Ok(());
        }

        // Jump'n'Run control (:3398-3403).
        let control_style = self
            .players
            .get(&controller)
            .map(|player| player.control.control_style)
            .unwrap_or(false);
        if has_controller && control_style {
            return self.auto_stop_direct_com(index, com, data);
        }

        // Control by procedure (:3405-3556).
        self.object_procedure_com(index, com)
    }

    /// `C4Menu::Control` (C4Menu.cpp:433-480) for a script-created object
    /// menu. Returns false only when no menu is active.
    fn object_menu_control(
        &mut self,
        index: usize,
        com: u8,
        data: i32,
    ) -> Result<bool, EngineError> {
        let Some(menu) = self.objects[index].state.menu.clone() else {
            return Ok(false);
        };
        let object_id = self.objects[index].id;
        match com {
            COM_MENU_ENTER => {
                if !self.enter_internal_context_put(index, &menu, false)?
                    && !self.enter_internal_context_exit(index, &menu)?
                {
                    self.menu_user_enter(object_id, false)?;
                }
            }
            COM_MENU_ENTER_ALL => {
                if !self.enter_internal_context_put(index, &menu, true)?
                    && !self.enter_internal_context_exit(index, &menu)?
                {
                    self.menu_user_enter(object_id, true)?;
                }
            }
            COM_MENU_CLOSE => {
                let auto_context_exit =
                    !menu.user_menu && menu.permanent && menu.identification == Value::Int(14);
                if self.close_object_menu(object_id, false)? && auto_context_exit {
                    // C4Object::AutoContextMenu's CloseCommand is invoked
                    // only for a control close (C4Menu.cpp:327-331), not
                    // when another menu force-replaces the context menu.
                    let owner = self.objects[index].state.owner;
                    self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
                    self.execute_object_command_now(object_id)?;
                }
            }
            COM_MENU_LEFT => {
                if !self.object_menu_step(object_id, &menu, -1)? {
                    let delta = if menu.selection - 1 < 0 {
                        menu.items.len() as i32 - 1 - menu.selection
                    } else {
                        -1
                    };
                    self.move_object_menu_selection(index, delta)?;
                }
            }
            COM_MENU_RIGHT => {
                if !self.object_menu_step(object_id, &menu, 1)? {
                    let delta = if menu.selection + 1 >= menu.items.len() as i32 {
                        -menu.selection
                    } else {
                        1
                    };
                    self.move_object_menu_selection(index, delta)?;
                }
            }
            COM_MENU_UP => {
                let columns = menu.columns;
                let mut delta = -columns;
                if menu.selection + delta < 0 && columns > 0 {
                    while menu.selection + delta + columns < menu.items.len() as i32 {
                        delta += columns;
                    }
                }
                self.move_object_menu_selection(index, delta)?;
            }
            COM_MENU_DOWN => {
                let columns = menu.columns;
                let mut delta = columns;
                if menu.selection + delta >= menu.items.len() as i32 && columns > 0 {
                    while menu.selection + delta - columns >= 0 {
                        delta -= columns;
                    }
                }
                self.move_object_menu_selection(index, delta)?;
            }
            COM_MENU_SELECT => {
                if !menu.items.is_empty() {
                    self.set_object_menu_selection(index, data & !C4MN_ADJUST_POSITION)?;
                }
            }
            COM_MENU_SHOW_TEXT => {
                if let Some(menu) = self.objects[index].state.menu.as_mut() {
                    menu.reveal_text();
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// Execute the engine-owned C4MN_Context Put row without routing its
    /// `PlayerObjectCommand` text through the script host-function table.
    /// C++ applies Put to every selected crew member, clamps the requested
    /// count to each inventory, and then synchronously executes the command
    /// object once (C4ObjectMenu.cpp:335-359; C4Player.cpp:1408-1423).
    fn enter_internal_context_put(
        &mut self,
        index: usize,
        menu: &crate::ObjectMenuState,
        right: bool,
    ) -> Result<bool, EngineError> {
        if menu.user_menu || !menu.permanent || menu.identification != Value::Int(14) {
            return Ok(false);
        }
        let Some(item) = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get(selection))
        else {
            return Ok(false);
        };
        if item.caption != "Put" {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        let Some(container) = self.objects[index].state.container else {
            return Ok(false);
        };
        let put_all = right && !item.command2.is_empty();
        self.player_context_put(owner, container, put_all)?;
        self.execute_object_command_now(object_id)?;
        Ok(true)
    }

    /// C4MN_Context's Exit row issues the player-wide Exit order, then
    /// executes it synchronously on the menu command object
    /// (C4ObjectMenu.cpp:426-433; C4ObjectCom.cpp:1013-1040).
    fn enter_internal_context_exit(
        &mut self,
        index: usize,
        menu: &crate::ObjectMenuState,
    ) -> Result<bool, EngineError> {
        if menu.user_menu || !menu.permanent || menu.identification != Value::Int(14) {
            return Ok(false);
        }
        let is_exit = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get(selection))
            .is_some_and(|item| item.symbol == crate::ObjectMenuSymbol::Exit);
        if !is_exit {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
        self.execute_object_command_now(object_id)?;
        Ok(true)
    }

    fn player_context_put(
        &mut self,
        owner: i32,
        container: ObjectId,
        put_all: bool,
    ) -> Result<(), EngineError> {
        let cursor = self.crew_cursor(owner);
        let mut crew = self.selected_crew(owner);
        if let Some(cursor) = cursor.filter(|cursor| !crew.contains(cursor)) {
            crew.push(cursor);
        }
        for crew_id in crew {
            if crew_id == container {
                continue;
            }
            let Some(index) = self.find_object_index(crew_id) else {
                continue;
            };
            if !self.objects[index].has_nonzero_status() {
                continue;
            }
            let mut contents = self.objects[index]
                .state
                .contents
                .iter()
                .copied()
                .filter(|object_id| {
                    self.find_object_index(*object_id)
                        .is_some_and(|index| self.objects[index].has_nonzero_status())
                })
                .collect::<Vec<_>>();
            if !put_all {
                contents.truncate(1);
            }
            if contents.is_empty() {
                continue;
            }
            let count = if put_all {
                i32::try_from(contents.len()).unwrap_or(i32::MAX)
            } else {
                0
            };
            self.object_command_to_obj(
                index,
                CommandId::Put,
                Some(container),
                None,
                count,
                0,
                0,
                PlayerObjectCommandMode::Set,
                true,
            )?;
        }
        Ok(())
    }

    /// `C4Menu::MoveSelection` (C4Menu.cpp:535-555): advance in fixed
    /// increments until a selectable item is found, without crossing the
    /// menu bounds.
    fn move_object_menu_selection(
        &mut self,
        index: usize,
        delta: i32,
    ) -> Result<bool, EngineError> {
        if delta == 0 {
            return Ok(false);
        }
        let Some(menu) = self.objects[index].state.menu.as_ref() else {
            return Ok(false);
        };
        let mut selection = menu.selection;
        loop {
            selection += delta;
            let Some(item) = usize::try_from(selection)
                .ok()
                .and_then(|selection| menu.items.get(selection))
            else {
                return Ok(false);
            };
            if item.selectable {
                break;
            }
        }
        self.set_object_menu_selection(index, selection)?;
        Ok(true)
    }

    /// clonk-rs divergence: offer a horizontal menu com to the script before
    /// it becomes a selection move. Once `Columns == 1` — which every style
    /// but `C4MN_Style_Normal` forces (C4Menu.cpp:359-365) —
    /// `C4Menu::Control` gives COM_MenuLeft/Right exactly the deltas
    /// COM_MenuUp/Down already carry (C4Menu.cpp:433-457), so the horizontal
    /// pair says nothing the vertical pair does not. A user menu's own
    /// command object may claim them by implementing `OnMenuStep(iDelta,
    /// pMenuObject)` and returning true. Everything else — a menu that is not
    /// script-created, more than one column, no such function, a falsy
    /// return, a script error — falls through to the shipped selection move
    /// unchanged, so this is inert for content that does not ask for it.
    /// Dispatch mirrors `set_object_menu_selection` below, which ports
    /// `C4ObjectMenu::OnSelectionChanged` (C4ObjectMenu.cpp:93-104).
    fn object_menu_step(
        &mut self,
        object_id: ObjectId,
        menu: &crate::ObjectMenuState,
        delta: i32,
    ) -> Result<bool, EngineError> {
        if !menu.user_menu || menu.columns != 1 {
            return Ok(false);
        }
        let args = vec![Value::Int(delta), compat::object_reference_value(object_id)];
        let handled = if menu.scenario_callbacks {
            self.call_scenario_script_value("OnMenuStep", &args)?
        } else {
            let Some(command_object) = menu.command_object else {
                return Ok(false);
            };
            let Some(command_index) =
                self.find_object_index(command_object)
                    .filter(|&command_index| {
                        self.definitions
                            .get(&self.objects[command_index].definition_id)
                            .is_some_and(|definition| definition.has_function("OnMenuStep"))
                    })
            else {
                return Ok(false);
            };
            tolerate_script_error(self.call_object_function(command_index, "OnMenuStep", args))?
        };
        Ok(handled.is_some_and(|value| value.as_bool()))
    }

    /// `C4Menu::SetSelection(..., fDoCalls=true)` +
    /// `C4ObjectMenu::OnSelectionChanged` (C4Menu.cpp:557-594;
    /// C4ObjectMenu.cpp:93-104).
    fn set_object_menu_selection(
        &mut self,
        index: usize,
        requested: i32,
    ) -> Result<(), EngineError> {
        let object_id = self.objects[index].id;
        let Some(mut menu) = self.objects[index].state.menu.clone() else {
            return Ok(());
        };
        let selectable = usize::try_from(requested)
            .ok()
            .and_then(|selection| menu.items.get(selection))
            .is_some_and(|item| item.selectable);
        if (requested == -1 && menu.items.is_empty()) || selectable {
            menu.selection = requested;
            self.objects[index].state.menu = Some(menu.clone());
        }
        if !menu.user_menu {
            return Ok(());
        }
        let args = vec![
            Value::Int(menu.selection),
            compat::object_reference_value(object_id),
        ];
        if menu.scenario_callbacks {
            let _ = self.call_scenario_script_value("OnMenuSelection", &args)?;
        } else if let Some(command_object) = menu.command_object {
            let Some(command_index) =
                self.find_object_index(command_object)
                    .filter(|&command_index| {
                        self.definitions
                            .get(&self.objects[command_index].definition_id)
                            .is_some_and(|definition| definition.has_function("OnMenuSelection"))
                    })
            else {
                return Ok(());
            };
            let _ = tolerate_script_error(self.call_object_function(
                command_index,
                "OnMenuSelection",
                args,
            ))?;
        }
        Ok(())
    }

    /// The `switch (GetProcedure())` tail of C4Object::DirectCom
    /// (C4Object.cpp:3406-3556).
    fn object_procedure_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        match self.object_procedure(index) {
            ActionProcedure::Walk => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
                COM_UP => {
                    self.object_com_up(index)?;
                }
                COM_DOWN_D => {
                    self.object_com_down_double(index)?;
                }
                COM_DIG_S => {
                    // (:3416-3421)
                    if self.object_com_dig(index)? {
                        let direction = self.objects[index].state.direction;
                        self.objects[index].state.command_direction = match direction {
                            Direction::Right => CommandDirection::DownRight,
                            _ => CommandDirection::DownLeft,
                        };
                    }
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Flight | ActionProcedure::Kneel | ActionProcedure::Throw => {
                match com {
                    COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                    COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                    COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
                    COM_THROW => {
                        self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    }
                    _ => {}
                }
            }
            ActionProcedure::Scale => match com {
                COM_LEFT => {
                    if self.objects[index].state.direction == Direction::Left {
                        self.object_com_movement(index, CommandDirection::Stop)?;
                    } else {
                        self.object_com_movement(index, CommandDirection::Left)?;
                        self.object_com_let_go(index, -1)?;
                    }
                }
                COM_RIGHT => {
                    if self.objects[index].state.direction == Direction::Right {
                        self.object_com_movement(index, CommandDirection::Stop)?;
                    } else {
                        self.object_com_movement(index, CommandDirection::Right)?;
                        self.object_com_let_go(index, 1)?;
                    }
                }
                COM_UP => self.object_com_movement(index, CommandDirection::Up)?,
                COM_DOWN => self.object_com_movement(index, CommandDirection::Down)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Hang => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_UP => self.object_com_movement(index, CommandDirection::Stop)?,
                COM_DOWN => {
                    self.object_com_let_go(index, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Dig => match com {
                COM_LEFT => {
                    // COMD_UpRight(2)..COMD_Left(7) rotates one step clockwise
                    // (:3468).
                    let com_dir = self.objects[index]
                        .state
                        .command_direction
                        .to_script_value();
                    if (2..=7).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir + 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_RIGHT => {
                    // COMD_Right(3)..COMD_UpLeft(8) rotates one step
                    // counter-clockwise (:3469).
                    let com_dir = self.objects[index]
                        .state
                        .command_direction
                        .to_script_value();
                    if (3..=8).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir - 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_DIG_S => {
                    // Dig mat 2 object request (:3472).
                    let data = self.objects[index].state.action.data;
                    self.objects[index].state.action.data = i32::from(data == 0);
                }
                _ => {}
            },
            ActionProcedure::Swim => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_UP => {
                    self.object_com_movement(index, CommandDirection::Up)?;
                    self.object_com_up(index)?;
                }
                COM_DOWN => self.object_com_movement(index, CommandDirection::Down)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                _ => {}
            },
            ActionProcedure::Bridge | ActionProcedure::Build | ActionProcedure::Chop => {
                if com == COM_DOWN {
                    self.object_com_stop(index)?;
                }
            }
            ActionProcedure::Fight => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                _ => {}
            },
            ActionProcedure::Push => self.object_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of DirectCom (C4Object.cpp:3506-3555).
    fn object_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        // New grab-control model: objects version >= 4,9,5,0 may overload
        // control of grabbing clonks (:3508-3518).
        let grab_control_overload = if let Some(target_index) = target_index {
            self.objects[target_index].state.controller = controller;
            self.definitions
                .get(&self.objects[target_index].definition_id)
                .is_none_or(|definition| definition.version_at_least([4, 9, 5, 0]))
        } else {
            false
        };
        // Call object control first in case it overloads (:3520-3523).
        if grab_control_overload {
            if let Some(target_index) = target_index {
                let clonk_id = self.objects[index].id;
                if self.object_call_control(target_index, controller, com, Some(clonk_id))? {
                    return Ok(());
                }
            }
        }
        // Clonk direct control (:3525-3549).
        match com {
            COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
            COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
            COM_UP => {
                // Target -> enter, else comdir up for target straightening
                // (:3529-3536).
                if self.object_com_enter(target_index)? {
                    self.object_com_movement(index, CommandDirection::Stop)?;
                } else {
                    self.object_com_movement(index, CommandDirection::Up)?;
                }
            }
            COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
            COM_DOWN_D => {
                self.object_com_ungrab(index)?;
            }
            COM_THROW_D => {
                // Avoid breaking objects with non-default ControlThrow
                // (:3539-3544): with the overload active and a target without
                // its own ControlThrow the double falls through to Throw.
                let target_has_control_throw = target_index
                    .map(|target_index| self.object_has_function(target_index, "ControlThrow"))
                    .unwrap_or(true);
                if grab_control_overload && !target_has_control_throw {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
            }
            COM_THROW => {
                self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
            }
            _ => {}
        }
        // Action target call control late for old objects (:3550-3553).
        // Re-read Action.Target because the hardcoded fallback may have
        // changed or cleared it before this call.
        if !grab_control_overload {
            if let Some(target_index) = self
                .objects
                .get(index)
                .and_then(|object| object.state.action.target)
                .and_then(|id| self.find_object_index(id))
            {
                let clonk_id = self.objects[index].id;
                let _ = self.object_call_control(target_index, controller, com, Some(clonk_id))?;
            }
        }
        Ok(())
    }

    /// `C4Object::AutoStopDirectCom` (C4Object.cpp:3559-3727) — the
    /// Jump'n'Run per-procedure fallbacks.
    fn auto_stop_direct_com(
        &mut self,
        index: usize,
        com: u8,
        _data: i32,
    ) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        match self.object_procedure(index) {
            ActionProcedure::Walk => match com {
                COM_UP => {
                    self.object_com_up(index)?;
                }
                COM_DOWN => {
                    // Inhibit ControlDownSingle on freshly grabbed objects
                    // (:3569-3573).
                    if self.object_com_down_double(index)? {
                        if let Some(player) = self.players.get_mut(&controller) {
                            player.control.last_com = i32::from(COM_NONE);
                        }
                    }
                }
                COM_DIG_S => {
                    self.object_com_dig(index)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Flight => match com {
                COM_THROW => {
                    // Drop when pressing left, right or down (:3584-3590).
                    let pressed = self
                        .players
                        .get(&controller)
                        .map(|player| player.control.pressed_coms)
                        .unwrap_or(0);
                    let drop_mask = (1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_DOWN);
                    if pressed & drop_mask != 0 {
                        self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                    } else {
                        self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    }
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Kneel | ActionProcedure::Throw => match com {
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Scale => match com {
                COM_LEFT => {
                    if self.objects[index].state.direction == Direction::Right {
                        self.object_com_let_go(index, -1)?;
                    } else {
                        self.auto_stop_update_com_dir(index)?;
                    }
                }
                COM_RIGHT => {
                    if self.objects[index].state.direction == Direction::Left {
                        self.object_com_let_go(index, 1)?;
                    } else {
                        self.auto_stop_update_com_dir(index)?;
                    }
                }
                COM_DIG => {
                    // (:3615; note the C++ fallthrough into COM_Throw's drop.)
                    let xdirf = if self.objects[index].state.direction == Direction::Left {
                        1
                    } else {
                        -1
                    };
                    self.object_com_let_go(index, xdirf)?;
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Hang => match com {
                COM_DOWN | COM_DIG => {
                    self.object_com_let_go(index, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Dig => match com {
                COM_THROW | COM_DIG => {
                    let data = self.objects[index].state.action.data;
                    self.objects[index].state.action.data = i32::from(data == 0);
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Swim => match com {
                COM_UP => {
                    self.auto_stop_update_com_dir(index)?;
                    self.object_com_up(index)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Bridge | ActionProcedure::Build | ActionProcedure::Chop => {
                if com == COM_DOWN {
                    self.object_com_stop(index)?;
                }
            }
            ActionProcedure::Fight => match com {
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Push => self.auto_stop_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of AutoStopDirectCom (C4Object.cpp:3668-3725).
    fn auto_stop_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        let grab_control_overload = target_index.is_some_and(|target_index| {
            self.definitions
                .get(&self.objects[target_index].definition_id)
                .is_none_or(|definition| definition.version_at_least([4, 9, 5, 0]))
        });
        if grab_control_overload {
            if let Some(target_index) = target_index {
                let clonk_id = self.objects[index].id;
                if self.object_call_control(target_index, controller, com, Some(clonk_id))? {
                    return Ok(());
                }
            }
        }
        match com {
            COM_UP => {
                if self.object_com_enter(target_index)? {
                    self.object_com_movement(index, CommandDirection::Stop)?;
                } else {
                    self.auto_stop_update_com_dir(index)?;
                }
            }
            COM_DOWN => {
                // C++ queries the three Down command slots and only ungrabs
                // when none is visible for this player's control style
                // (C4Object.cpp:3712-3721; C4Object.cpp:2938-2951).
                let target_has_down_command = target_index.is_some_and(|target_index| {
                    ["ControlDown", "ControlDownSingle", "ControlDownDouble"]
                        .iter()
                        .any(|function| {
                            self.object_control_command_is_visible(
                                target_index,
                                controller,
                                function,
                            )
                        })
                });
                if target_index.is_some() && !target_has_down_command {
                    self.object_com_ungrab(index)?;
                }
            }
            COM_DOWN_D => {
                self.object_com_ungrab(index)?;
            }
            COM_THROW_D => {
                let target_has_control_throw = target_index
                    .map(|target_index| self.object_has_function(target_index, "ControlThrow"))
                    .unwrap_or(true);
                if grab_control_overload && !target_has_control_throw {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
            }
            COM_THROW => {
                self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
            }
            _ => self.auto_stop_update_com_dir(index)?,
        }
        if !grab_control_overload {
            if let Some(target_index) = self
                .objects
                .get(index)
                .and_then(|object| object.state.action.target)
                .and_then(|id| self.find_object_index(id))
            {
                let clonk_id = self.objects[index].id;
                let _ = self.object_call_control(target_index, controller, com, Some(clonk_id))?;
            }
        }
        Ok(())
    }

    /// `C4Object::AutoStopUpdateComDir` (C4Object.cpp:3729-3741).
    fn auto_stop_update_com_dir(&mut self, index: usize) -> Result<(), EngineError> {
        let controller = self.objects[index].state.controller;
        let Some(player) = self.players.get(&controller) else {
            return Ok(());
        };
        if self.crew_cursor(controller) != Some(self.objects[index].id) {
            return Ok(());
        }
        let new_com_dir = coms_to_com_dir(player.control.pressed_coms);
        if self.objects[index].state.command_direction == new_com_dir {
            return Ok(());
        }
        if new_com_dir == CommandDirection::Stop
            && self.object_procedure(index) == ActionProcedure::Dig
        {
            self.object_com_stop(index)?;
            return Ok(());
        }
        self.object_com_movement(index, new_com_dir)
    }

    /// `C4Object::ContainedControl` (C4Object.cpp:3219-3305).
    pub(crate) fn object_contained_control(
        &mut self,
        index: usize,
        com: u8,
    ) -> Result<bool, EngineError> {
        let Some(container_id) = self.objects[index].state.container else {
            return Ok(false);
        };
        let Some(container_index) = self.find_object_index(container_id) else {
            return Ok(false);
        };
        // Check if object is about to exit a structure (:3223-3230).
        if (com == COM_LEFT || com == COM_RIGHT)
            && self.objects[index].commands.front_command_name() == Some("Exit")
            && self.objects[container_index].state.category & crate::CATEGORY_STRUCTURE != 0
        {
            return Ok(false);
        }
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let function = format!("Contained{}", com_name_raw(com));
        let callback_definition_id = self.objects[container_index].definition_id.clone();
        let sf = self.object_script_callback(container_index, &function);
        // New definitions may overload hardcoded controls; old definitions
        // receive the callback only after those controls have run
        // (C4Object.cpp:3246-3251,3284-3291).
        let call_sf_early = self
            .definitions
            .get(&self.objects[container_index].definition_id)
            .is_none_or(|definition| definition.version_at_least([4, 9, 1, 3]));
        let mut result = false;
        if call_sf_early {
            if let Some(sf) = sf.as_ref() {
                let clonk_ref = compat::object_reference_value(self.objects[index].id);
                let value = self.contained_direct_callback(
                    Some(container_index),
                    &callback_definition_id,
                    sf,
                    &[clonk_ref],
                )?;
                if compat::value_raw_truthy(&value) {
                    result = true;
                }
            }
            // AutoStopControl: notify container about the control update
            // (:3242-3249).
            self.contained_control_update(index, com, controller)?;
        }
        if result {
            return Ok(true);
        }

        // Hardcoded actions (:3253-3281).
        match com {
            COM_DOWN => {
                self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
            }
            COM_THROW_D => {
                // Avoid breaking objects with non-default ContainedThrow
                // (:3259-3265): only fall through when no such override.
                let container_index_now = self
                    .objects
                    .get(index)
                    .and_then(|object| object.state.container)
                    .and_then(|id| self.find_object_index(id));
                let has_contained_throw = container_index_now
                    .map(|idx| self.object_has_function(idx, "ContainedThrow"))
                    .unwrap_or(false);
                if !has_contained_throw {
                    let object_id = self.objects[index].id;
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    self.execute_object_command_now(object_id)?;
                }
            }
            COM_THROW => {
                // `PlayerObjectCommand(...) && ExecuteCommand()`
                // (C4Object.cpp:3280-3282):
                // execute the calling clonk's freshly queued command before
                // returning from ContainedControl.
                let object_id = self.objects[index].id;
                self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                self.execute_object_command_now(object_id)?;
            }
            COM_UP => {
                // Base buy menu (:3269-3274): ValidPlr(Contained->Base),
                // not hostile, BASEFUNC_Buy → ActivateMenu(C4MN_Buy).
                self.contained_base_menu(index, /* buy */ true)?;
            }
            COM_DIG => {
                // Base sell menu (:3275-3280): the BASEFUNC_Sell twin.
                self.contained_base_menu(index, /* buy */ false)?;
            }
            _ => {}
        }
        if !call_sf_early {
            if let Some(sf) = sf.as_ref() {
                let container_index = self
                    .objects
                    .get(index)
                    .and_then(|object| object.state.container)
                    .and_then(|id| self.find_object_index(id));
                let clonk_ref = compat::object_reference_value(self.objects[index].id);
                let _ = self.contained_direct_callback(
                    container_index,
                    &callback_definition_id,
                    sf,
                    &[clonk_ref],
                )?;
            }
            self.contained_control_update(index, com, controller)?;
        }
        // Take/Take2 (:3293-3302).
        if sf.is_none() || call_sf_early {
            match com {
                COM_LEFT => {
                    self.player_object_command(owner, CommandId::Take, None, 0, 0)?;
                }
                COM_RIGHT => {
                    self.player_object_command(owner, CommandId::Take2, None, 0, 0)?;
                }
                _ => {}
            }
        }
        Ok(true)
    }

    /// The base buy/sell menu arms of ContainedControl
    /// (C4Object.cpp:3269-3280): ValidPlr(Contained->Base), not hostile to
    /// the clonk's Owner, and the scenario's BASEFUNC bit set →
    /// ActivateMenu(C4MN_Buy/C4MN_Sell) on the clonk with the container as
    /// target. C4Object owns this permanent menu directly.
    fn contained_base_menu(&mut self, index: usize, buy: bool) -> Result<(), EngineError> {
        // Re-resolve the container: the early Contained{Com} script may
        // have moved the clonk.
        let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        else {
            return Ok(());
        };
        let base = self.objects[container_index].state.base;
        if !self.players.contains_key(&base) {
            return Ok(());
        }
        let owner = self.objects[index].state.owner;
        if self.players_hostile(owner, base) {
            return Ok(());
        }
        let enabled = if buy {
            self.base_buy_enabled
        } else {
            self.base_sell_enabled
        };
        if !enabled {
            return Ok(());
        }
        if buy {
            self.open_base_buy_menu(index, container_index)?;
        } else {
            self.open_base_sell_menu(index, container_index)?;
        }
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Construction): install the classic
    /// structure-knowledge menu directly on the owning crew
    /// (C4Object.cpp:1982-2005).
    pub(crate) fn open_construction_menu(&mut self, crew_index: usize) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;

        let crew_id = self.objects[crew_index].id;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let owner = self.objects[crew_index].state.owner;
        let Some(player) = self.players.get(&owner) else {
            return Ok(());
        };
        let player_name = player.name().to_string();
        let knowledge = player
            .knowledge_entries()
            .iter()
            .map(|(definition_id, _)| definition_id.clone())
            .collect::<Vec<_>>();
        let symbol_id = if self.definitions.contains_key("CXCN") {
            "CXCN".to_string()
        } else if self.definitions.contains_key("WKS1") {
            "WKS1".to_string()
        } else {
            String::new()
        };
        // C4Object::ActivateMenu initializes the shell before AddRefSym
        // resolves each row's potentially callback-driven components.
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("Player {player_name}|has no construction plans."),
            symbol_id,
            title_symbol: crate::ObjectMenuSymbol::default(),
            identification: Value::Int(1),
            style: 0,
            equal_item_height: false,
            permanent: false,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Components,
            extra_data: 0,
            internal_refill_token: 0,
            selection: -1,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: None,
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items: Vec::new(),
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });

        for definition_id in knowledge {
            let Some((name, description)) = self
                .definitions
                .get(&definition_id)
                .filter(|definition| definition.category() & crate::CATEGORY_STRUCTURE != 0)
                .map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                    )
                })
            else {
                continue;
            };
            let components = self
                .build_required_components(&definition_id, crew_id)?
                .into_iter()
                .map(|component| crate::ObjectMenuComponent {
                    definition_id: component.id,
                    count: component.count,
                })
                .collect();
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(1) && menu.command_object == Some(crew_id)
            }) else {
                return Ok(());
            };
            let select = menu.selection == -1;
            menu.items.push(crate::ObjectMenuItem {
                caption: format!("Construction: {name}"),
                info_caption: crate::normalize_menu_info_caption(description),
                command: format!("SetCommand(this, \"Construct\",,0,0,,{definition_id})"),
                command2: String::new(),
                count: C4MN_ITEM_NO_COUNT,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components,
                selectable: true,
                value: None,
                text_display_progress: -1,
            });
            if select {
                menu.selection = 0;
            }
        }
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Buy) plus the immediate
    /// C4ObjectMenu::SetRefillObject/Refill pass (C4Object.cpp:1919-1930;
    /// C4ObjectMenu.cpp:207-237).
    pub(crate) fn open_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_buy_menu(crew_index, base_index, false)
    }

    fn refill_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_buy_menu(crew_index, base_index, true)
    }

    fn build_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        continue_existing: bool,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let base_id = self.objects[base_index].id;
        let previous_selection = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| {
                menu.identification == Value::Int(4)
                    && (!continue_existing || menu.refill_object == Some(base_id))
            })
            .map(|menu| menu.selection);
        let base_player = self.objects[base_index].state.base;
        let base_owner = self.objects[base_index].state.owner;
        let material = self
            .players
            .get(&base_player)
            .map(|player| {
                player
                    .home_base_material_entries()
                    .iter()
                    .map(|(definition_id, count)| (definition_id.clone(), *count))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut items = Vec::new();
        for (definition_id, count) in material {
            let Some((definition_name, definition_description)) =
                self.definitions.get(&definition_id).map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                    )
                })
            else {
                continue;
            };
            let command = format!(
                "AppendCommand(this,\"Buy\",Object({}),1,0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                definition_id
            );
            let command2 = format!(
                "AppendCommand(this,\"Buy\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                count,
                definition_id
            );
            let value = self.definition_value_in_container_for_menu(
                crew_id,
                &definition_id,
                base_id,
                base_player,
            )?;
            items.push(crate::ObjectMenuItem {
                caption: format!("Buy {definition_name}"),
                info_caption: crate::normalize_menu_info_caption(&definition_description),
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components: Vec::new(),
                selectable: true,
                value: Some(value),
                text_display_progress: -1,
            });
        }
        // C4ObjectMenu rebuilds Buy rows with ClearItems(false), preserving
        // the numeric slot. C4Menu::AdjustSelection keeps it when valid and
        // otherwise walks backward to the final selectable row
        // (C4ObjectMenu.cpp:207-237; C4Menu.cpp:947-988,1014-1038).
        let selection = if items.is_empty() {
            -1
        } else {
            let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
            previous_selection.unwrap_or(0).clamp(0, last)
        };

        if continue_existing {
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(4) && menu.refill_object == Some(base_id)
            }) {
                menu.items = items;
                menu.selection = selection;
            }
            return Ok(());
        }

        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: "There is nothing to buy.".to_string(),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Buy { owner: base_owner },
            identification: Value::Int(4),
            style: 0,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            internal_refill_token: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: Some(base_id),
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// The row partition produced by `C4ObjectListIterator::GetNext` for an
    /// object menu: category-ineligible entries are invisible, same-ID chunks
    /// retain list order, and only `CanConcatPictureWith`-equal objects share
    /// a row (C4ObjectList.cpp:849-903).
    fn object_menu_picture_groups(
        &mut self,
        contents: &[ObjectId],
        category_mask: i32,
    ) -> Vec<(ObjectId, i32)> {
        internal_object_menu_picture_groups(
            &EngineInternalObjectMenuSource(self),
            contents,
            category_mask,
        )
        .into_iter()
        .map(|group| (group.representative, group.count))
        .collect()
    }

    /// The native C4ObjectMenu refill owns each internal row's picture at
    /// construction time. Keep the effective object graphics as immutable
    /// presentation inputs so a later tick cannot blank the symbol
    /// (`C4ObjectMenu.cpp:194-199,311-313,350-372`).
    fn native_object_menu_picture_snapshot(
        &self,
        object_id: ObjectId,
    ) -> Option<crate::ObjectMenuPictureSnapshot> {
        let index = self.find_object_index(object_id)?;
        let object = &self.objects[index];
        Some(crate::ObjectMenuPictureSnapshot {
            definition_id: object.definition_id.clone(),
            symbol_size: 35,
            base_graphics: object.state.base_graphics.clone(),
            graphics_overlays: object.state.graphics_overlays.clone(),
            blit_mode: object.state.blit_mode,
            color: object.state.color,
            color_modulation: object.state.color_modulation,
            picture_rect: object.state.picture_rect,
            rank: None,
        })
    }

    /// `C4ObjectList::ObjectCount(id)` counts every live same-ID content,
    /// independently of the category/picture group used for the visible row
    /// (C4ObjectList.cpp:320-329; C4ObjectMenu.cpp:266-267,317-319).
    fn live_contents_definition_count(&self, contents: &[ObjectId], definition_id: &str) -> i32 {
        let count = contents
            .iter()
            .filter_map(|candidate| self.find_object_index(*candidate))
            .filter(|&candidate| {
                let candidate = &self.objects[candidate];
                candidate.has_nonzero_status() && candidate.definition_id == definition_id
            })
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    fn live_contents_count(&self, contents: &[ObjectId]) -> i32 {
        let count = contents
            .iter()
            .filter_map(|candidate| self.find_object_index(*candidate))
            .filter(|&candidate| self.objects[candidate].has_nonzero_status())
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    /// `C4Object::GetValue(pInBase, iForPlr)` for the value cached on an
    /// Activate/Sell-menu row. Running the public host expression preserves
    /// CalcValue/CalcDefValue, construction scaling, CalcSellValue, side
    /// effects, and fail-safe script errors (C4ObjectMenu.cpp:190-201,
    /// 268-271).
    fn object_value_in_container_for_menu(
        &mut self,
        command_object: ObjectId,
        object_id: ObjectId,
        container_id: ObjectId,
        for_player: i32,
    ) -> Result<i32, EngineError> {
        let expression = format!(
            "GetValue(Object({}),0,Object({}),{})",
            object_id.as_u64(),
            container_id.as_u64(),
            for_player
        );
        let Some(command_index) = self.find_object_index(command_object) else {
            return Ok(0);
        };
        Ok(crate::tolerate_script_error(self.direct_exec_on_object(
            command_index,
            &expression,
            "ObjectMenuValue",
        ))?
        .and_then(|value| value.as_c4_int())
        .unwrap_or(0))
    }

    /// `C4Def::GetValue(pInBase, iForPlr)` for a Buy-menu row
    /// (C4ObjectMenu.cpp:230-233). The definition and base hooks execute on
    /// every refill before the row is appended.
    fn definition_value_in_container_for_menu(
        &mut self,
        command_object: ObjectId,
        definition_id: &str,
        container_id: ObjectId,
        for_player: i32,
    ) -> Result<i32, EngineError> {
        let expression = format!(
            "GetValue(0,{},Object({}),{})",
            definition_id,
            container_id.as_u64(),
            for_player
        );
        let Some(command_index) = self.find_object_index(command_object) else {
            return Ok(0);
        };
        Ok(crate::tolerate_script_error(self.direct_exec_on_object(
            command_index,
            &expression,
            "BuyMenuValue",
        ))?
        .and_then(|value| value.as_c4_int())
        .unwrap_or(0))
    }

    /// `ClearItems(false)` keeps the numeric slot. `checkIDSelection` first
    /// accepts that slot when its C4ID survived, otherwise finds the first row
    /// carrying the old C4ID; `AdjustSelection` supplies the numeric fallback
    /// (C4ObjectMenu.cpp:147-164; C4Menu.cpp:975-1017).
    fn refilled_object_menu_selection(
        items: &[crate::ObjectMenuItem],
        previous_selection: Option<i32>,
        selected_definition: Option<&str>,
    ) -> i32 {
        if items.is_empty() {
            return -1;
        }
        if let (Some(previous), Some(selected)) = (previous_selection, selected_definition) {
            if usize::try_from(previous)
                .ok()
                .and_then(|selection| items.get(selection))
                .is_some_and(|item| item.item_id == selected)
            {
                return previous;
            }
        }
        if let Some(selection) = selected_definition
            .and_then(|selected| items.iter().position(|item| item.item_id == selected))
            .and_then(|selection| i32::try_from(selection).ok())
        {
            return selection;
        }
        let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
        previous_selection.unwrap_or(0).clamp(0, last)
    }

    /// C4Object::ActivateMenu(C4MN_Sell) plus C4ObjectMenu's immediate
    /// refill over the base's stContents list (C4Object.cpp:1932-1943;
    /// C4ObjectMenu.cpp:238-277).
    pub(crate) fn open_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_sell_menu(crew_index, base_index, false)
    }

    fn refill_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_sell_menu(crew_index, base_index, true)
    }

    fn build_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        continue_existing: bool,
    ) -> Result<(), EngineError> {
        const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
        let crew_id = self.objects[crew_index].id;
        let base_id = self.objects[base_index].id;
        let (previous_selection, selected_definition) = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| {
                menu.identification == Value::Int(5)
                    && (!continue_existing || menu.refill_object == Some(base_id))
            })
            .map(|menu| {
                let selected_definition = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
                    .map(|item| item.item_id.clone());
                (Some(menu.selection), selected_definition)
            })
            .unwrap_or((None, None));
        let base_owner = self.objects[base_index].state.owner;
        let base_definition = self.objects[base_index].definition_id.clone();
        let contents = self.objects[base_index].state.contents.clone();
        let sell_category = crate::CATEGORY_STATIC_BACK
            | crate::CATEGORY_STRUCTURE
            | crate::CATEGORY_VEHICLE
            | crate::CATEGORY_OBJECT
            | CATEGORY_TRADE_LIVING;
        let mut items = Vec::new();

        for (item_id, count) in self.object_menu_picture_groups(&contents, sell_category) {
            let Some(item_index) = self.find_object_index(item_id) else {
                continue;
            };
            let definition_id = self.objects[item_index].definition_id.clone();
            let Some((definition_name, definition_description, no_sell)) =
                self.definitions.get(&definition_id).map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                        definition.no_sell(),
                    )
                })
            else {
                continue;
            };
            if no_sell != 0 {
                continue;
            }
            let all_count = self.live_contents_definition_count(&contents, &definition_id);
            let command = format!(
                "AppendCommand(this,\"Sell\",Object({}),1,0,Object({}),0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                item_id.as_u64(),
                definition_id
            );
            let command2 = format!(
                "AppendCommand(this,\"Sell\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                all_count,
                definition_id
            );
            let for_player = self
                .find_object_index(crew_id)
                .map(|index| self.objects[index].state.owner)
                .unwrap_or(-1);
            // C4ObjectMenu.cpp:258-263 renders Picture2Facet before
            // GetValue; capture it before the callback below.
            let picture_snapshot = self.native_object_menu_picture_snapshot(item_id);
            let value =
                self.object_value_in_container_for_menu(crew_id, item_id, base_id, for_player)?;
            items.push(crate::ObjectMenuItem {
                caption: format!("Sell {definition_name}"),
                info_caption: crate::normalize_menu_info_caption(&definition_description),
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot,
                picture_object: Some(item_id),
                components: Vec::new(),
                selectable: true,
                value: Some(value),
                text_display_progress: -1,
            });
        }

        // ClearItems(false) leaves C++'s numeric selection in place while
        // checkIDSelection restores the selected C4ID after refill. If that
        // C4ID vanished, AdjustSelection keeps the old slot when valid and
        // otherwise walks backward to the final row (C4ObjectMenu.cpp:
        // 147-164,238-275; C4Menu.cpp:975-1017).
        let selection = Self::refilled_object_menu_selection(
            &items,
            previous_selection,
            selected_definition.as_deref(),
        );
        let base_name = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());
        if continue_existing {
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(5) && menu.refill_object == Some(base_id)
            }) {
                menu.items = items;
                menu.selection = selection;
            }
            return Ok(());
        }
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("{} is empty.", base_name),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Sell { owner: base_owner },
            identification: Value::Int(5),
            style: 0,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            internal_refill_token: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: Some(base_id),
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Activate) plus its immediate contents
    /// refill (C4Object.cpp:1884-1918; C4ObjectMenu.cpp:170-205).
    pub(crate) fn open_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
    ) -> Result<(), EngineError> {
        self.set_activate_menu(crew_index, container_index, true, None)
    }

    pub(crate) fn initialize_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
    ) -> Result<(), EngineError> {
        self.set_activate_menu(crew_index, container_index, false, None)
    }

    pub(crate) fn set_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        continue_existing: bool,
        reused_menu_identity: Option<u64>,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let container_id = self.objects[container_index].id;
        let menu = build_activate_menu_state(
            &mut EngineInternalObjectMenuSource(self),
            crew_id,
            container_id,
            continue_existing,
            reused_menu_identity,
        )?;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        if self.find_object_index(container_id).is_none() {
            return Ok(());
        }
        self.objects[crew_index].state.menu = menu;
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Get/C4MN_Contents) plus the immediate
    /// contents refill (C4Object.cpp:1945-1959; C4ObjectMenu.cpp:279-326).
    pub(crate) fn open_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
    ) -> Result<(), EngineError> {
        self.set_container_contents_menu(crew_index, container_index, identification, true, None)
    }

    pub(crate) fn initialize_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
    ) -> Result<(), EngineError> {
        self.set_container_contents_menu(crew_index, container_index, identification, false, None)
    }

    pub(crate) fn set_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
        continue_existing: bool,
        reused_menu_identity: Option<u64>,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let container_id = self.objects[container_index].id;
        let menu = build_container_contents_menu_state(
            &mut EngineInternalObjectMenuSource(self),
            crew_id,
            container_id,
            identification,
            continue_existing,
            reused_menu_identity,
        )?;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = menu;
        Ok(())
    }

    /// The `PSF_ContainedControlUpdate` (`~ContainedUpdate`) notification for
    /// Jump'n'Run control (C4Script.h:74; C4Object.cpp:3256-3262,3300-3304).
    fn contained_control_update(
        &mut self,
        index: usize,
        com: u8,
        controller: i32,
    ) -> Result<(), EngineError> {
        if com & (COM_SINGLE | COM_DOUBLE) != 0 {
            return Ok(());
        }
        let Some(player) = self.players.get(&controller) else {
            return Ok(());
        };
        if !player.control.control_style {
            return Ok(());
        }
        let pressed = player.control.pressed_coms;
        let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        else {
            return Ok(());
        };
        let clonk_ref = compat::object_reference_value(self.objects[index].id);
        let args = [
            clonk_ref,
            Value::Int(coms_to_com_dir(pressed).to_script_value()),
            Value::Bool(pressed & (1 << COM_DIG) != 0),
            Value::Bool(pressed & (1 << COM_THROW) != 0),
        ];
        self.contained_call(container_index, "ContainedUpdate", &args)?;
        Ok(())
    }

    /// `C4Object::CallControl` (C4Object.cpp:3307-3325): the `Control{Com}`
    /// script override, C4Value-truthy, plus the Jump'n'Run ControlUpdate
    /// notification.
    fn object_call_control(
        &mut self,
        index: usize,
        controller: i32,
        com: u8,
        clonk_arg: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let function = format!("Control{}", com_name_raw(com));
        let args: Vec<Value> = clonk_arg
            .map(|id| vec![compat::object_reference_value(id)])
            .into_iter()
            .flatten()
            .collect();
        let value = self.contained_call(index, &function, &args)?;
        let result = compat::value_raw_truthy(&value);
        // ControlUpdate for Jump'n'Run control (:3313-3323).
        let (control_style, pressed) = self
            .players
            .get(&controller)
            .map(|player| (player.control.control_style, player.control.pressed_coms))
            .unwrap_or((false, 0));
        if control_style {
            let first = clonk_arg
                .map(compat::object_reference_value)
                .unwrap_or_else(|| compat::object_reference_value(self.objects[index].id));
            let args = [
                first,
                Value::Int(coms_to_com_dir(pressed).to_script_value()),
                Value::Bool(pressed & (1 << COM_DIG) != 0),
                Value::Bool(pressed & (1 << COM_THROW) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL2) != 0),
            ];
            self.contained_call(index, "ControlUpdate", &args)?;
        }
        Ok(result)
    }

    /// Fail-safe object script call used by the control chain: script
    /// errors log and the tick continues (C4AulExec fail-safe execution,
    /// C4AulExec.cpp:1318-1342). Missing functions return Nil like `Call`
    /// with the `~` prefix.
    fn contained_call(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        if self.objects[index].destroyed
            || self.objects[index].state.status == crate::ObjectStatus::Deleted
        {
            return Ok(Value::Nil);
        }
        self.contained_call_unchecked(index, function, args)
    }

    fn contained_call_unchecked(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let Some(definition) = self.definitions.get(&definition_id) else {
            return Ok(Value::Nil);
        };
        // C4Object::Call receives an already linked C4AulFunc pointer. A
        // missing failsafe callback returns nil before C4AulExec allocates a
        // context (C4AulExec.cpp:1318-1342; C4ObjectCom.cpp:48-61).
        if !definition.script.has_function(function) {
            return Ok(Value::Nil);
        }
        let library = definition.shared_action_library_handle();
        let object_id = self.objects[index].id;
        Ok(tolerate_script_error(self.call_movement_object_function(
            index,
            function,
            args,
            &library,
            object_id,
            &definition_id,
        ))?
        .unwrap_or(Value::Nil))
    }

    /// `Contained{Com}` is invoked through the `C4AulFunc *sf` captured by
    /// C4Object::ContainedControl, not through C4Object::Call. Preserve that
    /// exact function and raw receiver; ContainedUpdate and ordinary
    /// Control* calls use `contained_call` above and retain their Status gate
    /// (C4Object.cpp:3237-3255,3297-3302; C4AulExec.cpp:1610-1625).
    fn contained_direct_callback(
        &mut self,
        index: Option<usize>,
        definition_id: &DefinitionId,
        callback: &ScriptCallbackTarget,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let result = match index {
            Some(index) => self.call_direct_object_callback_from_definition(
                index,
                definition_id,
                callback,
                args.to_vec(),
            ),
            None => self.call_direct_definition_callback(definition_id, callback, args.to_vec()),
        };
        Ok(tolerate_script_error(result)?.unwrap_or(Value::Nil))
    }

    fn object_script_callback(&self, index: usize, function: &str) -> Option<ScriptCallbackTarget> {
        let definition = self
            .definitions
            .get(&self.objects.get(index)?.definition_id)?;
        let resolution = definition.script.resolve_function(function, false)?;
        Some(ScriptCallbackTarget::linked(function, resolution))
    }

    fn object_has_function(&self, index: usize, function: &str) -> bool {
        self.definitions
            .get(&self.objects[index].definition_id)
            .map(|definition| definition.script.has_function(function))
            .unwrap_or(false)
    }

    /// `DrawCommandQuery`'s function-presence and `Method=` filter
    /// (C4ScriptHost.cpp:95-118; C4Object.cpp:2938-2951). C4Aul functions
    /// default to `All`; an unknown Method value also falls back to `All`
    /// (C4AulLink.cpp:200; C4AulParse.cpp:355-367).
    fn object_control_command_is_visible(
        &self,
        index: usize,
        controller: i32,
        function: &str,
    ) -> bool {
        let Some(control_style) = self
            .players
            .get(&controller)
            .map(|player| player.control.control_style)
        else {
            return false;
        };
        let Some(function) = self
            .definitions
            .get(&self.objects[index].definition_id)
            .and_then(|definition| definition.script.functions().get(function))
        else {
            return false;
        };
        let method = function.description.as_deref().and_then(|description| {
            description.split('|').find_map(|segment| {
                let (key, value) = segment.split_once('=')?;
                key.trim()
                    .eq_ignore_ascii_case("Method")
                    .then(|| value.trim())
            })
        });
        match method {
            Some(method) if method.eq_ignore_ascii_case("None") => false,
            Some(method) if method.eq_ignore_ascii_case("Classic") => !control_style,
            Some(method) if method.eq_ignore_ascii_case("JumpAndRun") => control_style,
            _ => true,
        }
    }

    fn object_procedure(&self, index: usize) -> ActionProcedure {
        let Some(definition) = self.definitions.get(&self.objects[index].definition_id) else {
            return ActionProcedure::Undefined;
        };
        let library = definition.action_library();
        let action = &self.objects[index].state.action;
        if library.is_idle_state(action) {
            return ActionProcedure::Undefined;
        }
        library.procedure_for_entry(&action.name, action.act_map_index)
    }

    // ---- Contents shifting (C4Object.cpp:5751-5797) -----------------------

    /// `C4Object::ShiftContents` (C4Object.cpp:5751-5775): walk First->Next
    /// (or Last->Prev with `shift_back`) for the first present item the
    /// current front cannot concat-picture with, using the full definition,
    /// color, graphics, name, and overlay rules; select it via
    /// DirectComContents.
    fn object_shift_contents(
        &mut self,
        index: usize,
        shift_back: bool,
        do_calls: bool,
    ) -> Result<bool, EngineError> {
        let contents = self.objects[index].state.contents.clone();
        let present_contents: Vec<ObjectId> = contents
            .into_iter()
            .filter(|candidate_id| {
                self.find_object_index(*candidate_id)
                    .is_some_and(|candidate| self.objects[candidate].has_nonzero_status())
            })
            .collect();
        let Some(front_id) = present_contents.first().copied() else {
            return Ok(false);
        };
        let Some(front) = self.object_snapshot(front_id) else {
            return Ok(false);
        };
        let mut candidates: Vec<ObjectId> = present_contents[1..].to_vec();
        if shift_back {
            candidates.reverse();
        }
        for candidate_id in candidates {
            let Some(candidate) = self.object_snapshot(candidate_id) else {
                continue;
            };
            if !self.can_concat_picture_with(&front, &candidate) {
                // Object different: shift to this (C4Object.cpp:5768).
                self.object_direct_com_contents(index, candidate_id, do_calls)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `C4Object::DirectComContents` (C4Object.cpp:5777-5797): the
    /// ~ControlContents veto, the cyclic rotation to the front, and the
    /// ~Selection callback whose falsy return plays the Grab sound. The
    /// context-menu refill (:5792-5795) is app-side presentation.
    fn object_direct_com_contents(
        &mut self,
        index: usize,
        target_id: ObjectId,
        do_calls: bool,
    ) -> Result<(), EngineError> {
        // Safety: present and contained in this object (:5780). Both
        // Status=1 (Normal) and Status=2 (Inactive) are truthy in C++.
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(());
        };
        if !self.objects[target_index].has_nonzero_status()
            || self.objects[target_index].state.container != Some(self.objects[index].id)
        {
            return Ok(());
        }
        // Desired object already at front? (:5782)
        let front = self.objects[index]
            .state
            .contents
            .iter()
            .copied()
            .find(|candidate_id| {
                self.find_object_index(*candidate_id)
                    .is_some_and(|candidate| self.objects[candidate].has_nonzero_status())
            });
        if front == Some(target_id) {
            return Ok(());
        }
        // Select object via script? (:5784-5786)
        let target_definition = self.objects[target_index].definition_id.clone();
        if do_calls {
            let veto = self.contained_call(
                index,
                "ControlContents",
                &[Value::C4Id(target_definition.as_str().to_string())],
            )?;
            if compat::value_raw_truthy(&veto) {
                return Ok(());
            }
        }
        // Default action: the cyclic relink (C4ObjectList::ShiftContents,
        // C4ObjectList.cpp:815-833) — a no-op if the id left the list.
        let contents = &mut self.objects[index].state.contents;
        let Some(position) = contents.iter().position(|id| *id == target_id) else {
            return Ok(());
        };
        contents.rotate_left(position);
        // Selection sound (:5790): falsy ~Selection(container) on the new
        // front plays "Grab" at the container.
        if do_calls {
            let container_ref = compat::object_reference_value(self.objects[index].id);
            let selected = self.contained_call(target_index, "Selection", &[container_ref])?;
            if !compat::value_raw_truthy(&selected) {
                let container_id = self.objects[index].id;
                self.emit_audio_command(crate::AudioCommand::PlaySound {
                    name: "Grab".to_string(),
                    target: Some(container_id),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                    target_position: None,
                });
            }
        }
        Ok(())
    }

    // ---- ObjectCom* helpers (C4ObjectCom.cpp) -----------------------------

    /// `ObjectComMovement` (C4ObjectCom.cpp:220-237).
    fn object_com_movement(
        &mut self,
        index: usize,
        com_dir: CommandDirection,
    ) -> Result<(), EngineError> {
        self.objects[index].state.command_direction = com_dir;
        let owner = self.objects[index].state.owner;
        let self_id = self.objects[index].id;
        // Selected crew follows the moving cursor (:224).
        self.player_object_command(owner, CommandId::Follow, Some(self_id), 0, 0)?;
        // Direct turnaround if standing still (:226-235).
        let procedure = self.object_procedure(index);
        if self.objects[index].fixed_velocity.x.val() == 0
            && matches!(procedure, ActionProcedure::Walk | ActionProcedure::Hang)
        {
            // Native calls `cObj->SetDir(...)` here, not a bare assignment:
            // SetDir runs the current action's TurnAction through
            // SetActionByName before writing the facing, and rejects idle or
            // out-of-range directions first (C4Object.cpp:4237-4253). Going
            // through the trailing assignment alone left the object in its old
            // action (clonk-org/clonk-rs#1124).
            let turn = match com_dir {
                CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
                    Some(Direction::Left)
                }
                CommandDirection::Right
                | CommandDirection::UpRight
                | CommandDirection::DownRight => Some(Direction::Right),
                _ => None,
            };
            if let Some(direction) = turn {
                let definition_id = self.objects[index].definition_id.clone();
                self.set_exec_action_direction(index, &definition_id, direction)?;
            }
        }
        Ok(())
    }

    /// `ObjectComStop` (C4ObjectCom.cpp:239-245): cease action, then stand.
    fn object_com_stop(&mut self, index: usize) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        self.object_com_stop_action(index, &definition_id)
    }

    /// C4Command::Grab's direct ObjectComStop call. Unlike the older shared
    /// engine helper, C++ uses ordinary SetActionByName transitions here,
    /// so a current NoOtherAction may block Idle and Walk.
    fn object_com_stop_for_grab(&mut self, index: usize) -> Result<bool, EngineError> {
        let actor_id = self.objects[index].id;
        let definition_id = self.objects[index].definition_id.clone();
        let _ = self.action_with_calls(index, &definition_id, "Idle")?;

        let Some(index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        self.objects[index].state.command_direction = CommandDirection::Stop;
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Walk")? {
            return Ok(false);
        }
        if let Some(index) = self.find_object_index(actor_id) {
            let object = &mut self.objects[index];
            object.fixed_velocity = FixedVec2::ZERO;
            object.state.velocity = Vector2::ZERO;
        }
        Ok(true)
    }

    /// `ObjectComUp` (C4ObjectCom.cpp:335-351): entrance first, then jump.
    fn object_com_up(&mut self, index: usize) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(self_id))
        {
            if target_ocf & ocf::ENTRANCE != 0 {
                return self.player_object_command(owner, CommandId::Enter, Some(target_id), 0, 0);
            }
        }
        if self.object_procedure(index) == ActionProcedure::Walk {
            return self.player_object_command(owner, CommandId::Jump, None, 0, 0);
        }
        Ok(false)
    }

    /// `ObjectComDig` (C4ObjectCom.cpp:353-362): CanDig gate + Dig action,
    /// with the native localized object message on either failure path.
    pub(crate) fn object_com_dig(&mut self, index: usize) -> Result<bool, EngineError> {
        let actor_id = self.objects[index].id;
        let physical = self.object_physical(index);
        let definition_id = self.objects[index].definition_id.clone();
        if physical.can_dig == 0 || !self.action_with_calls(index, &definition_id, "Dig")? {
            let name = self.object_message_name(actor_id);
            let text = self.object_no_dig_resource_string.replacen("%s", &name, 1);
            self.game_msg_object(actor_id, text);
            return Ok(false);
        }
        // ObjectActionDig resets the Dig2Object request (:143).
        self.objects[index].state.action.data = 0;
        Ok(true)
    }

    /// First nonzero-Status entry returned by `Contents.GetObject()`.
    fn first_live_content_id(&self, index: usize) -> Option<ObjectId> {
        self.objects[index]
            .state
            .contents
            .iter()
            .copied()
            .find(|object_id| {
                self.find_object_index(*object_id).is_some_and(|index| {
                    !self.objects[index].destroyed
                        && self.objects[index].state.status != crate::ObjectStatus::Deleted
                })
            })
    }

    fn dig_double_physical_backing(&mut self, index: usize) -> DigDoublePhysicalBacking {
        let object_id = self.objects[index].id;
        if self.objects[index].state.temporary_physical.is_some() {
            DigDoublePhysicalBacking::Temporary
        } else if self.crew_object_infos.contains_key(&object_id)
            || self.objects[index].state.info_physical.is_some()
            || self.objects[index].state.crew_member
        {
            let linked_info = self.crew_object_infos.contains_key(&object_id);
            let physical = if linked_info {
                self.object_physical(index)
            } else {
                self.objects[index]
                    .state
                    .info_physical
                    .or_else(|| {
                        self.definitions
                            .get(&self.objects[index].definition_id)
                            .map(|definition| *definition.physical())
                    })
                    .unwrap_or_default()
            };
            if linked_info && self.use_fair_crew() {
                DigDoublePhysicalBacking::FairCrew(physical)
            } else {
                DigDoublePhysicalBacking::Info(physical)
            }
        } else {
            DigDoublePhysicalBacking::Definition(self.objects[index].definition_id.clone())
        }
    }

    fn physical_from_dig_double_backing(
        &self,
        index: usize,
        backing: &DigDoublePhysicalBacking,
    ) -> PhysicalInfo {
        match backing {
            DigDoublePhysicalBacking::Temporary => self.objects[index]
                .state
                .temporary_physical
                .unwrap_or_default(),
            DigDoublePhysicalBacking::FairCrew(physical) => *physical,
            DigDoublePhysicalBacking::Info(initial) => self.objects[index]
                .state
                .info_physical
                .or(self.objects[index].retired_info_physical)
                .unwrap_or(*initial),
            DigDoublePhysicalBacking::Definition(definition_id) => self
                .definitions
                .get(definition_id)
                .map(|definition| *definition.physical())
                .unwrap_or_default(),
        }
    }

    /// `ObjectComDigDouble` (C4ObjectCom.cpp:531-571) — "activation":
    /// contents Activate, linekit construction, chop, line pickup, then own
    /// Activate.
    fn object_com_dig_double(&mut self, index: usize) -> Result<(), EngineError> {
        let self_id = self.objects[index].id;
        let physical_backing = self.dig_double_physical_backing(index);
        // Contents activation — first contents object only (:537-539).
        if let Some(contents_id) = self.first_live_content_id(index) {
            if let Some(contents_index) = self.find_object_index(contents_id) {
                let clonk_ref = compat::object_reference_value(self_id);
                let value = self.contained_call(contents_index, "Activate", &[clonk_ref])?;
                if compat::value_raw_truthy(&value) {
                    return Ok(());
                }
            }
        }

        let Some(index) = self.find_object_index(self_id) else {
            return Ok(());
        };
        // Re-read the first content after Activate. A leading LNKT always
        // consumes DigDouble even when line construction fails (:542-547).
        let first_contents = self.first_live_content_id(index);
        if first_contents.is_some_and(|contents_id| {
            self.find_object_index(contents_id)
                .is_some_and(|contents_index| self.objects[contents_index].definition_id == "LNKT")
        }) {
            let _ = self.object_com_line_construction(index)?;
            return Ok(());
        }

        // Chop (:549-558).
        let physical = self.physical_from_dig_double_backing(index, &physical_backing);
        if physical.can_chop != 0 && self.object_procedure(index) != ActionProcedure::Swim {
            let position = self.objects[index].state.position;
            if let Some((_, target_id, target_ocf)) =
                self.at_object(position, ocf::CHOP, Some(self_id))
            {
                if target_ocf & ocf::CHOP != 0 {
                    let owner = self.objects[index].state.owner;
                    self.player_object_command(owner, CommandId::Chop, Some(target_id), 0, 0)?;
                    return Ok(());
                }
            }
        }

        // Empty-hand line pickup follows Chop and has an outer physical/
        // structure precheck before the helper repeats its live gate
        // (C4ObjectCom.cpp:559-567).
        if self
            .physical_from_dig_double_backing(index, &physical_backing)
            .can_construct
            != 0
            && self.first_live_content_id(index).is_none()
        {
            let position = self.objects[index].state.position;
            if self
                .at_object(position, ocf::LINE_CONSTRUCT, Some(self_id))
                .is_some_and(|(_, _, object_ocf)| object_ocf & ocf::LINE_CONSTRUCT != 0)
                && self.object_com_line_construction(index)?
            {
                return Ok(());
            }
        }

        // Own activation call (:569-570).
        let self_ref = compat::object_reference_value(self_id);
        if let Some(index) = self.find_object_index(self_id) {
            self.contained_call(index, "Activate", &[self_ref])?;
        }
        Ok(())
    }

    /// First C++ master-list object whose live `Connect` action targets the
    /// supplied endpoint (`C4Game::FindObject`, C4Game.cpp:1391-1419).
    fn find_connect_line_index(
        &self,
        target: ObjectId,
        definition_id: Option<&str>,
    ) -> Option<usize> {
        self.exec_list.iter().rev().find_map(|object_id| {
            let index = self.find_object_index(*object_id)?;
            let object = &self.objects[index];
            (!object.destroyed
                && object.state.status.is_active()
                && self.object_ocf_at_index(index) != 0
                && definition_id.is_none_or(|id| object.definition_id == id)
                && object.state.action.name == "Connect"
                && (object.state.action.target == Some(target)
                    || object.state.action.target2 == Some(target)))
            .then_some(index)
        })
    }

    fn play_line_construction_sound(&mut self, name: &str, clonk_id: ObjectId) {
        self.emit_audio_command(crate::AudioCommand::PlaySound {
            name: name.to_owned(),
            target: Some(clonk_id),
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
            target_position: None,
        });
    }

    pub(crate) fn object_message_name(&self, object_id: ObjectId) -> String {
        self.find_object_index(object_id)
            .map(|index| &self.objects[index])
            .and_then(|object| {
                object
                    .state
                    .custom_name
                    .clone()
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        self.crew_object_infos
                            .get(&object_id)
                            .map(|info| info.name.clone())
                    })
                    .or_else(|| {
                        self.definitions
                            .get(&object.definition_id)
                            .map(|definition| definition.name().to_owned())
                    })
            })
            .unwrap_or_default()
    }

    /// `GameMsgObject` after its caller resolves the active `LoadResStr` text.
    /// Ordering and target replacement are simulation-visible.
    fn game_msg_object(&mut self, target: ObjectId, text: String) {
        // C4GameMessageList::New replaces prior messages before its deleted
        // target guard, so a failed GameMsgObject still performs the clear.
        self.messages.clear_for_object(target);
        let target_live = self.find_object_index(target).is_some_and(|index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        });
        if !target_live {
            return;
        }
        self.messages.add_message(MessageSpec {
            kind: message::MessageKind::Target,
            text,
            target: Some(target),
            player: None,
            offset: Vector2::ZERO,
            color: 0xffff_ffff,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
    }

    /// `ObjectComLineConstruction` (C4ObjectCom.cpp:379-529): stand and
    /// physical gate, pickup without a kit, finish an attached line, or
    /// create a new one.
    fn object_com_line_construction(&mut self, clonk_index: usize) -> Result<bool, EngineError> {
        // ObjectComLineConstruction enters Stand even when the following
        // physical gate rejects construction (C4ObjectCom.cpp:384-390).
        let clonk_id = self.objects[clonk_index].id;
        let clonk_definition = self.objects[clonk_index].definition_id.clone();
        self.objects[clonk_index].state.command_direction = CommandDirection::Stop;
        if self.action_with_calls(clonk_index, &clonk_definition, "Walk")? {
            if let Some(clonk_index) = self.find_object_index(clonk_id) {
                let clonk = &mut self.objects[clonk_index];
                clonk.fixed_velocity = FixedVec2::ZERO;
                clonk.state.velocity = Vector2::ZERO;
            }
        }
        let Some(clonk_index) = self.find_object_index(clonk_id) else {
            return Ok(false);
        };
        if self.object_physical(clonk_index).can_construct == 0 {
            let clonk_name = self.object_message_name(clonk_id);
            self.game_msg_object(clonk_id, format!("{clonk_name} cannot create lines."));
            return Ok(false);
        }

        let position = self.objects[clonk_index].state.position;
        let linekit_id = self.objects[clonk_index]
            .state
            .contents
            .iter()
            .copied()
            .find(|linekit_id| {
                self.find_object_index(*linekit_id).is_some_and(|index| {
                    let linekit = &self.objects[index];
                    !linekit.destroyed
                        && linekit.state.status != crate::ObjectStatus::Deleted
                        && self.object_ocf_at_index(index) != 0
                        && linekit.definition_id == "LNKT"
                })
            });

        // Line pickup (:392-427).
        let Some(linekit_id) = linekit_id else {
            let collection_limit = self
                .definitions
                .get(&self.objects[clonk_index].definition_id)
                .map_or(0, crate::Definition::collection_limit);
            let contents_count = self.objects[clonk_index]
                .state
                .contents
                .iter()
                .filter(|object_id| {
                    self.find_object_index(**object_id).is_some_and(|index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    })
                })
                .count();
            if crate::collection_limit_reached(collection_limit, contents_count) {
                return Ok(false);
            }

            let Some((_, structure_id, structure_ocf)) =
                self.at_object(position, ocf::LINE_CONSTRUCT, Some(clonk_id))
            else {
                return Ok(false);
            };
            if structure_ocf & ocf::LINE_CONSTRUCT == 0 {
                return Ok(false);
            }
            let Some(line_index) = self.find_connect_line_index(structure_id, None) else {
                return Ok(false);
            };
            let first = self.objects[line_index].state.action.target;
            let second = self.objects[line_index].state.action.target2;
            let endpoint_is_linekit = |engine: &Self, endpoint: Option<ObjectId>| {
                endpoint
                    .and_then(|endpoint| engine.find_object_index(endpoint))
                    .is_some_and(|index| engine.objects[index].definition_id == "LNKT")
            };
            if endpoint_is_linekit(self, first) || endpoint_is_linekit(self, second) {
                self.play_line_construction_sound("Error", clonk_id);
                let line_name = self.object_message_name(self.objects[line_index].id);
                self.game_msg_object(
                    clonk_id,
                    format!("{line_name} is not fixed at the other end."),
                );
                return Ok(false);
            }
            if !self.definitions.contains_key("LNKT") {
                return Ok(false);
            }

            let line_id = self.objects[line_index].id;
            let line_owner = self.objects[line_index].state.owner;
            let clonk_layer = self.objects[clonk_index].state.layer;
            let mut linekit_config = crate::SpawnConfig::new("LNKT")
                .with_position(Vector2::new(50, 50))
                .with_owner(line_owner);
            if let Some(layer) = clonk_layer {
                linekit_config = linekit_config.with_layer(layer);
            }
            let linekit_id =
                self.spawn_object_with_initial_lifecycle(linekit_config, Some(clonk_id))?;
            let Some(linekit_id) = linekit_id else {
                return Ok(false);
            };
            if self.try_object_enter_with_reject_collect(linekit_id, clonk_id, true)?
                != ObjectEnterOutcome::Entered
            {
                let _ = self.assign_object_removal(linekit_id)?;
                return Ok(false);
            }

            self.play_line_construction_sound("Connect", clonk_id);
            if let Some(line_index) = self.find_object_index(line_id) {
                if self.objects[line_index].state.action.target == Some(structure_id) {
                    self.objects[line_index].state.action.target = Some(linekit_id);
                }
                if self.objects[line_index].state.action.target2 == Some(structure_id) {
                    self.objects[line_index].state.action.target2 = Some(linekit_id);
                }
            }
            let line_name = self.object_message_name(line_id);
            let structure_name = self.object_message_name(structure_id);
            self.game_msg_object(
                structure_id,
                format!("{line_name} disconnected|from {structure_name}."),
            );
            return Ok(true);
        };
        let Some(linekit_index) = self.find_object_index(linekit_id) else {
            return Ok(false);
        };

        let active_line = self.find_connect_line_index(linekit_id, None);

        let Some((structure_index, structure_id, structure_ocf)) =
            self.at_object(position, ocf::LINE_CONSTRUCT, Some(clonk_id))
        else {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(
                clonk_id,
                if active_line.is_some() {
                    "Connection not possible.".to_owned()
                } else {
                    "Cannot create a new line here.".to_owned()
                },
            );
            return Ok(false);
        };
        if structure_ocf & ocf::LINE_CONSTRUCT == 0 {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(
                clonk_id,
                if active_line.is_some() {
                    "Connection not possible.".to_owned()
                } else {
                    "Cannot create a new line here.".to_owned()
                },
            );
            return Ok(false);
        }

        if let Some(line_index) = active_line {
            let first = self.objects[line_index].state.action.target;
            let second = self.objects[line_index].state.action.target2;
            if first == Some(structure_id) || second == Some(structure_id) {
                self.play_line_construction_sound("Connect", clonk_id);
                let line_id = self.objects[line_index].id;
                let line_name = self.object_message_name(line_id);
                self.game_msg_object(structure_id, format!("{line_name} disconnected."));
                let _ = self.assign_object_removal(line_id)?;
                return Ok(true);
            }

            let line_type = self
                .definitions
                .get(&self.objects[line_index].definition_id)
                .map(|definition| definition.line())
                .unwrap_or_default();
            let line_connect = self
                .definitions
                .get(&self.objects[structure_index].definition_id)
                .map(|definition| definition.line_connect())
                .unwrap_or_default();
            let connect_ok = match line_type {
                1 => {
                    line_connect
                        & (crate::LINE_CONNECT_POWER_INPUT | crate::LINE_CONNECT_POWER_OUTPUT)
                        != 0
                }
                2 => line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0,
                3 => line_connect & crate::LINE_CONNECT_LIQUID_INPUT != 0,
                _ => return Ok(false),
            };
            if !connect_ok {
                self.play_line_construction_sound("Error", clonk_id);
                let line_name = self.object_message_name(self.objects[line_index].id);
                let structure_name = self.object_message_name(structure_id);
                self.game_msg_object(
                    structure_id,
                    format!("{line_name} cannot be connected|to {structure_name}."),
                );
                return Ok(false);
            }

            self.play_line_construction_sound("Connect", clonk_id);
            if first == Some(linekit_id) {
                self.objects[line_index].state.action.target = Some(structure_id);
            }
            if second == Some(linekit_id) {
                self.objects[line_index].state.action.target2 = Some(structure_id);
            }
            // Bare Exit() uses the default zero position/motion. Its return
            // is ignored; AssignRemoval still follows even if a callback
            // re-enters the kit (C4ObjectCom.cpp:479-480).
            if let Some(previous) = self.objects[linekit_index].state.container {
                let _ = self.exit_object_at_position_with_zero_motion(
                    linekit_id,
                    previous,
                    Vector2::ZERO,
                    0,
                )?;
            }
            let _ = self.assign_object_removal(linekit_id)?;
            let line_name = self.object_message_name(self.objects[line_index].id);
            let structure_name = self.object_message_name(structure_id);
            self.game_msg_object(
                structure_id,
                format!("{line_name} conntected|to {structure_name}"),
            );
            return Ok(true);
        }

        let line_connect = self
            .definitions
            .get(&self.objects[structure_index].definition_id)
            .map(|definition| definition.line_connect())
            .unwrap_or_default();
        let has_connected_line = |engine: &Self, definition_id: &str| {
            engine
                .find_connect_line_index(structure_id, Some(definition_id))
                .is_some()
        };
        let line_definition = if line_connect & crate::LINE_CONNECT_LIQUID_PUMP != 0
            && !has_connected_line(self, "SPIP")
        {
            Some("SPIP")
        } else if line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0
            && !has_connected_line(self, "DPIP")
        {
            Some("DPIP")
        } else if line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0 {
            Some("PWRL")
        } else {
            None
        };
        let Some(line_definition) = line_definition else {
            self.play_line_construction_sound("Error", clonk_id);
            self.game_msg_object(clonk_id, "Cannot create a new line here.".to_owned());
            return Ok(false);
        };
        let owner = self.objects[clonk_index].state.owner;
        let created = self.create_line_object(line_definition, owner, structure_id, linekit_id)?;
        if let Some(line_id) = created {
            self.play_line_construction_sound("Connect", clonk_id);
            let line_name = self.object_message_name(line_id);
            self.game_msg_object(structure_id, format!("New|{line_name}."));
        }
        Ok(created.is_some())
    }

    /// `ObjectComDownDouble` (C4ObjectCom.cpp:573-589): build or grab what
    /// is at the object's position.
    fn object_com_down_double(&mut self, index: usize) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::CONSTRUCT | ocf::GRAB, Some(self_id))
        {
            if target_ocf & ocf::CONSTRUCT != 0 {
                self.player_object_command(owner, CommandId::Build, Some(target_id), 0, 0)?;
                return Ok(true);
            }
            if target_ocf & ocf::GRAB != 0 {
                self.player_object_command(owner, CommandId::Grab, Some(target_id), 0, 0)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComLetGo` (C4ObjectCom.cpp:310-314): jump off a wall/ceiling.
    fn object_com_let_go(&mut self, index: usize, xdirf: i32) -> Result<bool, EngineError> {
        self.object_action_jump(index, itofix(xdirf), crate::C4Fixed::from_raw(0), true)
    }

    /// `ObjectComGrab` (C4ObjectCom.cpp:247-259): ordinary, non-forced Push
    /// followed by the two ordered script notifications and the live
    /// controller hand-off between them.
    fn object_com_grab(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(false);
        };
        if self.object_procedure(actor_index) != ActionProcedure::Walk {
            return Ok(false);
        }
        let definition_id = self.objects[actor_index].definition_id.clone();
        if !self.action_with_target_and_calls(actor_index, &definition_id, "Push", target_id)? {
            return Ok(false);
        }

        // ObjectActionPush's Start/Abort callbacks precede the explicit
        // Grab callback. A removed actor cannot execute the latter.
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let _ = tolerate_script_error(self.call_object_function(
            actor_index,
            "Grab",
            vec![compat::object_reference_value(target_id), Value::Bool(true)],
        ))?;

        // C++ checks both Status fields only after Grab. The callback may
        // remove either object or change the actor's Controller; propagate
        // the live post-callback value before calling Grabbed.
        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let Some(target_index) = self.find_object_index(target_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(true);
        };
        let controller = self.objects[actor_index].state.controller;
        self.objects[target_index].state.controller = controller;
        let _ = tolerate_script_error(self.call_object_function(
            target_index,
            "Grabbed",
            vec![compat::object_reference_value(actor_id), Value::Bool(true)],
        ))?;
        Ok(true)
    }

    /// C4Command::Grab's full live sequence (C4Command.cpp:667-716).
    /// ObjectComStop may run callbacks before the At test; ObjectComLetGo
    /// and RejectGrabbed likewise precede ObjectComGrab.
    pub(crate) fn execute_grab_command(
        &mut self,
        actor_id: ObjectId,
        target_id: ObjectId,
    ) -> Result<(), EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(());
        };
        if self.objects[actor_index].destroyed
            || self.objects[actor_index].state.status == crate::ObjectStatus::Deleted
        {
            return Ok(());
        }

        let (offset_x, offset_y) = self.objects[actor_index]
            .commands
            .pending_grab_offsets(target_id)
            .unwrap_or((0, 0));

        let mut stopped_for_grab = false;
        if matches!(
            self.object_procedure(actor_index),
            ActionProcedure::Build | ActionProcedure::Chop
        ) {
            stopped_for_grab = true;
            let _ = self.object_com_stop_for_grab(actor_index)?;
        }

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };
        if self.object_procedure(actor_index) == ActionProcedure::Dig {
            stopped_for_grab = true;
            let _ = self.object_com_stop_for_grab(actor_index)?;
        }

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };

        // ObjectComStop callbacks can install a Push action. C++ performs
        // this recheck before the null-target and At branches.
        if self.object_procedure(actor_index) == ActionProcedure::Push {
            let _ = self.objects[actor_index]
                .commands
                .resolve_grab_attempt(target_id, false);
            let _ = self.objects[actor_index].commands.push_front(
                CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub),
            );
            return Ok(());
        }

        if stopped_for_grab
            && self.objects[actor_index]
                .commands
                .fail_pending_grab_if_target_cleared(target_id)
        {
            return Ok(());
        }

        let target_at_actor = self
            .find_object_index(target_id)
            .filter(|&index| {
                !self.objects[index].destroyed
                    && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    && self.objects[index].state.container.is_none()
                    && self.objects[index].state.ocf & ocf::ALL != 0
            })
            .is_some_and(|target_index| {
                self.objects[actor_index].state.container.is_none()
                    && self
                        .object_shape_rect(&self.objects[target_index])
                        .contains_point(
                            self.objects[actor_index].state.position.x,
                            self.objects[actor_index].state.position.y,
                        )
            });

        if !target_at_actor {
            let target_retained = self.objects[actor_index]
                .commands
                .resolve_grab_attempt(target_id, false)
                .unwrap_or(true);
            if target_retained {
                if let Some(target_index) = self.find_object_index(target_id) {
                    let position = self.objects[target_index].state.position;
                    let _ = self.objects[actor_index].commands.push_front(
                        CommandRequest::new(CommandId::MoveTo)
                            .with_tx(Some(position.x.wrapping_add(offset_x)))
                            .with_ty(Some(position.y.wrapping_add(offset_y)))
                            .with_update_interval(50)
                            .with_mode(CommandMode::SilentSub),
                    );
                }
            }
            return Ok(());
        }

        if matches!(
            self.object_procedure(actor_index),
            ActionProcedure::Scale | ActionProcedure::Hang
        ) {
            let xdirf = if self.objects[actor_index].state.direction == Direction::Left {
                1
            } else {
                -1
            };
            let _ = self.object_com_let_go(actor_index, xdirf)?;
        }

        let rejected = match self.find_object_index(target_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) {
            Some(target_index) => tolerate_script_error(self.call_object_function(
                target_index,
                "RejectGrabbed",
                vec![compat::object_reference_value(actor_id)],
            ))?
            .is_some_and(|value| value.as_bool()),
            None => false,
        };

        let Some(actor_index) = self.find_object_index(actor_id).filter(|&index| {
            !self.objects[index].destroyed
                && self.objects[index].state.status != crate::ObjectStatus::Deleted
        }) else {
            return Ok(());
        };
        let target_retained = self.objects[actor_index]
            .commands
            .resolve_grab_attempt(target_id, rejected)
            .unwrap_or(true);
        if rejected {
            return Ok(());
        }

        self.objects[actor_index].state.command_direction = CommandDirection::Stop;
        if target_retained {
            let _ = self.object_com_grab(actor_id, target_id)?;
        }
        Ok(())
    }

    /// `C4Command::Jump` followed by `ObjectComJump` (C4Command.cpp:
    /// 1056-1067; C4ObjectCom.cpp:280-307). This stays live because
    /// ObjectActionJump synchronously invokes the object's OnActionJump hook.
    pub(crate) fn execute_jump_command(
        &mut self,
        object_id: ObjectId,
        tx: i32,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Tx==0 is the C++ sentinel: do not reinterpret it as world x=0.
        if tx != 0 {
            let x = self.objects[index].state.position.x;
            let direction = if tx < x {
                Some(Direction::Left)
            } else if tx > x {
                Some(Direction::Right)
            } else {
                None
            };
            if let Some(direction) = direction {
                let definition_id = self.objects[index].definition_id.clone();
                self.set_command_action_direction(index, &definition_id, direction)?;
            }
        }
        let _ = self.object_com_jump(index)?;
        // C4Command::Jump calls Finish(true) only after ObjectComJump and its
        // synchronous OnActionJump callback return (C4Command.cpp:1064-1067).
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index]
                .commands
                .finish_front_if(CommandId::Jump);
        }
        Ok(())
    }

    /// `ObjectComJump` (C4ObjectCom.cpp:280-307): predict a deep-liquid
    /// landing from the shape's bottom vertex before falling back to the
    /// script-overridable regular jump.
    pub(crate) fn object_com_jump(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Walk {
            return Ok(false);
        }
        // Native GetPhysical may run the lazy fair-crew fill before any of
        // GetCon, Action.ComDir, or Action.Dir are read (:286-294).
        let physical = self.object_physical(index);
        let launch = crate::command::object_com_jump_launch(
            self.objects[index].state.construction,
            physical,
            self.objects[index].state.command_direction,
            self.objects[index].state.direction,
        );
        // ObjectComJump reads pObj->Shape.ContactDensity, not Def->Shape
        // (C4ObjectCom.cpp:297-305). SetContactDensity therefore changes the
        // dive gate independently for every live object.
        let contact_density = self.objects[index].state.contact_density;
        if contact_density > 25
            && self.object_com_jump_hits_liquid(index, launch)
            && self.object_action_dive(index, launch.x, launch.y)?
        {
            return Ok(true);
        }
        self.object_action_jump(index, launch.x, launch.y, true)
    }

    /// `SimFlightHitsLiquid` (C4Movement.cpp:657-670), including the
    /// ten-frame escape when the bottom vertex already starts in water.
    fn object_com_jump_hits_liquid(&self, index: usize, launch: FixedVec2) -> bool {
        let Some(object) = self.objects.get(index) else {
            return false;
        };
        // Despite the name, C4Shape::GetBottomVertex selects the CNAT_Bottom
        // vertex with the smallest VtxY (C4Shape.cpp:445-455).
        let bottom = object
            .state
            .vertices
            .iter()
            .filter(|vertex| vertex.cnat & crate::CNAT_BOTTOM != 0)
            .min_by_key(|vertex| vertex.y);
        let mut position = object.fixed_position;
        if let Some(bottom) = bottom {
            position.x += bottom.x;
            position.y += bottom.y;
        }
        let mut velocity = launch;
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let solid_masks = self.live_movement_solid_masks();
        let density_at =
            |x, y| crate::movement_density_at(landscape, &self.materials, &solid_masks, None, x, y);
        let width = landscape.width() as i32;
        let height = landscape.estimated_height();
        let gravity = self.physics.gravity_as_c4fixed();
        let liquid = |density| (25..50).contains(&density);

        if liquid(density_at(
            crate::math::fixtoi(position.x),
            crate::math::fixtoi(position.y),
        )) && !sim_flight_to_density(
            &mut position,
            &mut velocity,
            0,
            24,
            10,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        if !sim_flight_to_density(
            &mut position,
            &mut velocity,
            25,
            100,
            -1,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        let x = crate::math::fixtoi(position.x);
        let y = crate::math::fixtoi(position.y);
        liquid(density_at(x, y)) && liquid(density_at(x, y + 9))
    }

    /// `ObjectActionDive` (C4ObjectCom.cpp:63-72): unlike a regular jump,
    /// Dive has no OnActionJump callback.
    fn object_action_dive(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
    ) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Dive")? {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(crate::math::fixtoi(xdir), crate::math::fixtoi(ydir));
        object.state.mobile = true;
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        object.state.t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
    }

    /// `ObjectActionJump` (C4ObjectCom.cpp:48-61): the scripted OnActionJump
    /// override, then the hardcoded Jump action with launch velocity.
    pub(crate) fn object_action_jump(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
        by_com: bool,
    ) -> Result<bool, EngineError> {
        let args = [
            Value::Int(crate::math::fixtoi_prec(xdir, 100)),
            Value::Int(crate::math::fixtoi_prec(ydir, 100)),
            Value::Bool(by_com),
        ];
        let value = self.contained_call(index, "OnActionJump", &args)?;
        if compat::value_raw_truthy(&value) {
            return Ok(true);
        }
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Jump")? {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(crate::math::fixtoi(xdir), crate::math::fixtoi(ydir));
        object.state.mobile = true;
        // Unstick from ground: attach-values were already determined for
        // this frame (:58-59).
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        object.state.t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
    }

    /// `ObjectComEnter` for the pushed target (C4ObjectCom.cpp:316-333):
    /// the vehicle enters the entrance at its own position via a plain
    /// SetCommand.
    fn object_com_enter(&mut self, target_index: Option<usize>) -> Result<bool, EngineError> {
        let Some(target_index) = target_index else {
            return Ok(false);
        };
        if self
            .definitions
            .get(&self.objects[target_index].definition_id)
            .is_some_and(|definition| definition.no_push_enter() != 0)
        {
            return Ok(false);
        }
        let position = self.objects[target_index].state.position;
        let target_id = self.objects[target_index].id;
        if let Some((_, entrance_id, entrance_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(target_id))
        {
            if entrance_ocf & ocf::ENTRANCE != 0 {
                self.set_object_command(
                    target_index,
                    CommandRequest::new(CommandId::Enter)
                        .with_target(Some(entrance_id))
                        .with_mode(CommandMode::Base),
                    false,
                )?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComDrop` (C4ObjectCom.cpp:640-676): calculate the live shape-
    /// relative exit and fixed launch, run the full Exit callback sequence,
    /// then arm collection delay and release any current Push action.
    pub(crate) fn object_com_drop(
        &mut self,
        actor_id: ObjectId,
        object_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(actor_index) = self.find_object_index(actor_id) else {
            return Ok(false);
        };
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };

        let throw_force = math::val_by_physical(400, self.object_physical(actor_index).throw);
        let procedure = self.object_procedure(actor_index);
        let command_direction = self.objects[actor_index].state.command_direction;
        let actor_xdir = self.objects[actor_index].fixed_velocity.x;
        let actor_position = self.objects[actor_index].state.position;
        let actor_shape = self.objects[actor_index]
            .current_shape_rect()
            .unwrap_or_default();
        let object_shape = self.objects[object_index]
            .current_shape_rect()
            .unwrap_or_default();

        let com_dir_like = |sample: CommandDirection| {
            let com = command_direction.to_script_value();
            let sample = sample.to_script_value();
            com == sample || com % 8 + 1 == sample || com == sample % 8 + 1
        };
        let hangling_or_swimming =
            matches!(procedure, ActionProcedure::Hang | ActionProcedure::Swim);
        let mut throw_direction = 0;
        let mut right = 0;
        let mut outpos_reduction = 1;
        if procedure != ActionProcedure::Scale {
            if com_dir_like(CommandDirection::Left) {
                throw_direction = -1;
                if actor_xdir < math::fixed10(15) && !hangling_or_swimming {
                    outpos_reduction -= 1;
                }
            }
            if com_dir_like(CommandDirection::Right) {
                throw_direction = 1;
                right = 1;
                if actor_xdir > -math::fixed10(15) && !hangling_or_swimming {
                    outpos_reduction -= 1;
                }
            }
        }

        let edge = actor_shape
            .x
            .wrapping_add(actor_shape.width.wrapping_mul(right));
        let exit_position = Vector2::new(
            actor_position.x.wrapping_add(
                edge.wrapping_mul(i32::from(throw_direction != 0))
                    .wrapping_mul(outpos_reduction),
            ),
            actor_position
                .y
                .wrapping_add(actor_shape.y)
                .wrapping_add(actor_shape.height)
                .wrapping_sub(object_shape.y.wrapping_add(object_shape.height)),
        );
        let exit_velocity = FixedVec2::new(throw_force * throw_direction, C4Fixed::ZERO);

        // ObjectComDrop intentionally ignores Exit's boolean: callback
        // re-entry still proceeds to NoCollectDelay and ObjectComUnGrab.
        let _ = self.exit_object_for_drop(object_id, exit_position, exit_velocity)?;

        if let Some(actor_index) = self.find_object_index(actor_id) {
            self.objects[actor_index].state.no_collect_delay = 2;
            self.refresh_object_ocf(actor_index);
        }
        if let Some(actor_index) = self.find_object_index(actor_id) {
            let _ = self.object_com_ungrab(actor_index)?;
        }
        Ok(true)
    }

    /// The `C4Object::Exit` slice used by ObjectComDrop. The old parent is
    /// refreshed before BoundsCheck while the moving object's own OCF/menu
    /// remain stale; requested motion is installed before Ejection and
    /// Departure (C4Object.cpp:1513-1563).
    fn exit_object_for_drop(
        &mut self,
        object_id: ObjectId,
        target: Vector2,
        velocity: FixedVec2,
    ) -> Result<bool, EngineError> {
        let Some(object_index) = self.find_object_index(object_id) else {
            return Ok(false);
        };
        let Some(previous) = self.objects[object_index].state.container else {
            return Ok(false);
        };
        self.exit_object_at_position_with_full_motion(
            object_id,
            previous,
            target,
            0,
            velocity,
            C4Fixed::ZERO,
        )
    }

    /// `ObjectComUnGrab` (C4ObjectCom.cpp:261-278): stand up and release the
    /// grab with the Grab/Grabbed script notifications.
    pub(crate) fn object_com_ungrab(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Push {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let target = self.objects[index].state.action.target;
        if !self.object_action_stand_live(object_id)? {
            return Ok(false);
        }
        if !self.close_object_menu(object_id, false)? {
            return Ok(false);
        }
        if let Some(index) = self.find_object_index(object_id) {
            let target_ref = target
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil);
            self.contained_call(index, "Grab", &[target_ref, Value::Bool(false)])?;
            let actor_has_status = self
                .find_object_index(object_id)
                .is_some_and(|index| self.objects[index].has_nonzero_status());
            if actor_has_status {
                if let Some(target_index) = target
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| self.objects[index].has_nonzero_status())
                {
                    let self_ref = compat::object_reference_value(object_id);
                    self.contained_call(target_index, "Grabbed", &[self_ref, Value::Bool(false)])?;
                }
            }
        }
        Ok(true)
    }

    // ---- Player command routing -------------------------------------------

    /// `PlayerObjectCommand` (C4ObjectCom.cpp:1013-1040) +
    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): route a control
    /// command to the selected crew (and always the cursor), with the
    /// classic down-double throw→drop conversion.
    #[doc(hidden)]
    pub fn player_object_command(
        &mut self,
        owner: i32,
        mut command: CommandId,
        target: Option<ObjectId>,
        tx: i32,
        ty: i32,
    ) -> Result<bool, EngineError> {
        let Some(player) = self.players.get_mut(&owner) else {
            return Ok(false);
        };
        // Adjust for old-style keyboard throw/drop control (:1018-1019).
        let ranged = matches!(command, CommandId::Throw | CommandId::Drop);
        if command == CommandId::Throw {
            let mut convert_to_drop = false;
            // Drop on down-down-throw (classic, :1024-1033).
            if player.control.last_com_down_double > 0 {
                convert_to_drop = true;
                player.control.last_com = i32::from(COM_DOWN | COM_DOUBLE);
                player.control.last_com_down_double = C4_DOUBLE_CLICK;
            }
            // Jump'n'Run: drop on combined Down+Throw (:1034-1035).
            if player.control.control_style && player.control.pressed_coms & (1 << COM_DOWN) != 0 {
                convert_to_drop = true;
            }
            if convert_to_drop {
                command = CommandId::Drop;
            }
        }
        let mode = if ranged {
            PlayerObjectCommandMode::Add
        } else {
            PlayerObjectCommandMode::Set
        };
        self.player_crew_object_command(owner, command, target, None, tx, ty, 0, mode, ranged)
    }

    /// `C4MouseControl::ButtonUpDragMoving`: issue one independent carryable
    /// Drop/Throw command per locally selected object. The first packet uses
    /// C4P_Command_Set and every later packet uses C4P_Command_Append, so each
    /// selected crew member handles every object in mouse-list order
    /// (C4MouseControl.cpp:1171-1201; C4Player.cpp:1397-1450).
    pub fn player_mouse_drag_objects<I>(
        &mut self,
        owner: i32,
        command: CommandId,
        objects: I,
        position: Vector2,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner)
            || !matches!(command, CommandId::Drop | CommandId::Throw)
        {
            return Ok(false);
        }
        let mut mode = PlayerObjectCommandMode::Set;
        let mut issued = false;
        for target in objects {
            self.player_crew_object_command(
                owner,
                command,
                Some(target),
                None,
                position.x,
                position.y,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Control-modified carryable drag onto an `OCF_Container`: each packet
    /// is `Put(Target=container, Target2=dragged object, X=Y=0)`. The first
    /// object replaces the crew command stack and the rest append in mouse
    /// selection order; Shift makes the first packet append as well
    /// (C4MouseControl.cpp:742-768,1171-1219).
    pub fn player_mouse_drag_put<I>(
        &mut self,
        owner: i32,
        objects: I,
        container: ObjectId,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for object in objects {
            self.player_crew_object_command(
                owner,
                CommandId::Put,
                Some(container),
                Some(object),
                0,
                0,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Issue ButtonUpDragMoving's vehicle commands. Every selected Grab=1
    /// object receives `PushTo(Target=vehicle, Target2=optional container)`
    /// at the release coordinates; the first packet is Set and later packets
    /// Append, while Shift makes the first packet Append too
    /// (C4MouseControl.cpp:1171-1227).
    pub fn player_mouse_drag_vehicles<I>(
        &mut self,
        owner: i32,
        vehicles: I,
        position: Vector2,
        put_target: Option<ObjectId>,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for vehicle in vehicles {
            self.player_crew_object_command(
                owner,
                CommandId::PushTo,
                Some(vehicle),
                put_target,
                position.x,
                position.y,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Mouse `C4CMD_Context`: unlike ordinary PlayerObjectCommand, the
    /// clicked object occupies Target2 while Target remains null, and Add
    /// mode does not apply the ±15 cursor range (C4MouseControl.cpp:
    /// 1253-1260; C4Player.cpp:1397-1451).
    pub fn player_context_command(
        &mut self,
        owner: i32,
        target: ObjectId,
    ) -> Result<bool, EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        self.player_crew_object_command(
            owner,
            CommandId::Context,
            None,
            Some(target),
            0,
            0,
            0,
            PlayerObjectCommandMode::Add,
            false,
        )
    }

    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): apply to all
    /// selected crew in cursor range except the target, then always to the
    /// cursor. `ranged` mirrors C4P_Command_Add|C4P_Command_Range.
    fn player_crew_object_command(
        &mut self,
        owner: i32,
        command: CommandId,
        target: Option<ObjectId>,
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        data: i32,
        mode: PlayerObjectCommandMode,
        ranged: bool,
    ) -> Result<bool, EngineError> {
        if self.is_owner_eliminated(owner) {
            return Ok(false);
        }
        // C4Player::ObjectCommand clears ShowStartup before it commits the
        // selection toggle or dispatches commands to crew.
        if let Some(player) = self.players.get_mut(&owner) {
            player.hide_startup();
        }
        self.player_update_selection_toggle_status(owner)?;
        let cursor = self.crew_cursor(owner);
        let cursor_position = cursor
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.position);
        let selected = self.selected_crew(owner);
        let mut cursor_processed = false;
        for crew_id in selected {
            if Some(crew_id) == cursor {
                cursor_processed = true;
            }
            if Some(crew_id) == target {
                continue;
            }
            let Some(index) = self.find_object_index(crew_id) else {
                continue;
            };
            if !self.objects[index].has_nonzero_status() {
                continue;
            }
            if ranged {
                // C4P_Command_Range: within ±15 of the cursor (:1412).
                let Some(cursor_position) = cursor_position else {
                    continue;
                };
                let position = self.objects[index].state.position;
                if (position.x - cursor_position.x).abs() > 15
                    || (position.y - cursor_position.y).abs() > 15
                {
                    continue;
                }
            }
            self.object_command_to_obj(index, command, target, target2, tx, ty, data, mode, true)?;
        }
        // Always apply to cursor, even if it's not selected (:1436-1439).
        if let Some(cursor_id) = cursor {
            if !cursor_processed && Some(cursor_id) != target {
                if let Some(index) = self.find_object_index(cursor_id) {
                    if self.objects[index].has_nonzero_status() {
                        self.object_command_to_obj(
                            index, command, target, target2, tx, ty, data, mode, true,
                        )?;
                    }
                }
            }
        }
        Ok(true)
    }

    /// `C4Player::ObjectCommand2Obj` (C4Player.cpp:1445-1451): Add-mode
    /// commands push in front of the stack, Set-mode commands replace it.
    /// The Set path is `C4Object::SetCommand` with fControl
    /// (C4Object.cpp:3923-3981): clear, then the soft menu close, then the
    /// `ControlCommand` script overload before the hardcoded push.
    fn object_command_to_obj(
        &mut self,
        index: usize,
        command: CommandId,
        target: Option<ObjectId>,
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        data: i32,
        mode: PlayerObjectCommandMode,
        f_control: bool,
    ) -> Result<(), EngineError> {
        let request = CommandRequest::new(command)
            .with_target(target)
            .with_target2(target2)
            .with_tx((tx != 0).then_some(tx))
            .with_ty((ty != 0).then_some(ty))
            .with_data(CommandData::Integer(data))
            .with_mode(CommandMode::Base);
        match mode {
            PlayerObjectCommandMode::None => return Ok(()),
            PlayerObjectCommandMode::Add => {
                // C4P_Command_Add → AddCommand(..., fAppend=false): push front
                // without clearing (C4Command.cpp AddCommand semantics).
                self.objects[index]
                    .apply_command_operations([CommandOperation::PushFront(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Append => {
                // C4P_Command_Append → AddCommand(..., fAppend=true): retain
                // the independent command sequence in list order.
                self.objects[index].apply_command_operations([CommandOperation::PushBack(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Set => {}
        }
        self.set_object_command(index, request, f_control)
    }

    /// `C4Object::SetCommand` for a fully parsed request. Only menu closing
    /// and the command object's own ControlCommand overload are gated by
    /// `f_control`; contained/pushed vehicle overloads run for every entry
    /// point (C4Object.cpp:3939-3983).
    pub(crate) fn set_object_command(
        &mut self,
        index: usize,
        request: CommandRequest,
        f_control: bool,
    ) -> Result<(), EngineError> {
        // SetCommand: decrement NoCollectDelay (:3941-3942), then clear the
        // stack (:3943).
        self.objects[index].apply_command_operations([
            CommandOperation::DecrementNoCollectDelay,
            CommandOperation::Clear,
        ]);
        let object_id = self.objects[index].id;
        if f_control {
            // Close menu — soft: `if (!CloseMenu(false)) return;`
            // (C4Object.cpp:3944-3946). A MenuQueryCancel denial aborts the
            // SetCommand with the stack already cleared.
            if !self.close_object_menu(object_id, false)? {
                return Ok(());
            }
        }
        // The optional menu query may run script, so re-resolve the index.
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Script overload (:3935-3942): `ControlCommand(name, target, tx,
        // ty, target2, data)`.
        let tx = request
            .tx_definition
            .as_ref()
            .map(|id| Value::C4Id(id.as_str().to_string()))
            .or_else(|| request.tx.map(Value::Int))
            .unwrap_or(Value::Int(0));
        let data = match &request.data {
            CommandData::Integer(value) => *value,
            CommandData::Text(_) | CommandData::None => 0,
        };
        let args = [
            Value::String(request.id.to_name().to_string().into()),
            request
                .target
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            tx,
            Value::Int(request.ty.unwrap_or(0)),
            request
                .target2
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            Value::Int(data),
        ];
        if f_control {
            let overloaded = self
                .contained_call(index, "ControlCommand", &args)
                .map(|value| compat::value_raw_truthy(&value))?;
            if overloaded {
                return Ok(());
            }
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Inside vehicle control overload (:3947-3961): the container's
        // ControlCommand with the clonk appended in slot 7.
        if let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        {
            let inside = self
                .definitions
                .get(&self.objects[container_index].definition_id)
                .is_some_and(|definition| {
                    definition.vehicle_control() & crate::VEHICLE_CONTROL_INSIDE != 0
                });
            if inside {
                let controller = self.objects[index].state.controller;
                self.objects[container_index].state.controller = controller;
                let mut vehicle_args = args.to_vec();
                vehicle_args.push(compat::object_reference_value(object_id));
                let consumed = self
                    .contained_call(container_index, "ControlCommand", &vehicle_args)
                    .map(|value| compat::value_raw_truthy(&value))?;
                if consumed {
                    return Ok(());
                }
            }
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Outside vehicle control overload (:3962-3974): the pushed
        // target's ControlCommand, plain six args.
        if self.object_procedure(index) == ActionProcedure::Push {
            if let Some(target_index) = self.objects[index]
                .state
                .action
                .target
                .and_then(|id| self.find_object_index(id))
            {
                let outside = self
                    .definitions
                    .get(&self.objects[target_index].definition_id)
                    .is_some_and(|definition| {
                        definition.vehicle_control() & crate::VEHICLE_CONTROL_OUTSIDE != 0
                    });
                if outside {
                    let controller = self.objects[index].state.controller;
                    self.objects[target_index].state.controller = controller;
                    let consumed = self
                        .contained_call(target_index, "ControlCommand", &args)
                        .map(|value| compat::value_raw_truthy(&value))?;
                    if consumed {
                        return Ok(());
                    }
                }
            }
        }
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index].apply_command_operations([CommandOperation::PushFront(request)]);
        }
        Ok(())
    }

    /// Native `SetCommand(C4CMD_Exit)` with the default `fControl=false`:
    /// clear and replace the stack without the menu/own-object control arms,
    /// while retaining the unconditional inside/outside vehicle overloads.
    pub(crate) fn set_plain_exit_command(&mut self, index: usize) -> Result<(), EngineError> {
        self.set_object_command(
            index,
            CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionSpec, ActionState, CrewInfoLink, CrewObjectInfo, Definition, MovementProfile,
        ObjectSnapshot, PhysicalInfo, PhysicsSettings, PlayerConfig, PlayerStatus, SpawnConfig,
    };
    use std::{collections::HashMap, rc::Rc};

    #[track_caller]
    fn test_definition(id: impl Into<String>, name: impl Into<String>, source: &str) -> Definition {
        Definition::from_script(id, name, source).test_value()
    }

    trait TestValueExt<T> {
        fn test_value(self) -> T;
    }

    impl<T> TestValueExt<T> for Option<T> {
        #[track_caller]
        fn test_value(self) -> T {
            Option::expect(self, "direct-com test value exists")
        }
    }

    impl<T, E: std::fmt::Debug> TestValueExt<T> for Result<T, E> {
        #[track_caller]
        fn test_value(self) -> T {
            Result::expect(self, "direct-com test operation succeeds")
        }
    }

    trait TestEngineExt {
        fn register_test_definition(&mut self, definition: Definition);
        fn register_test_player(&mut self, player: PlayerConfig);
        fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
        fn test_object_index(&self, id: ObjectId) -> usize;
        fn call_test_object_function(
            &mut self,
            index: usize,
            function: &str,
            args: Vec<Value>,
        ) -> Value;
    }

    impl TestEngineExt for Engine {
        #[track_caller]
        fn register_test_definition(&mut self, definition: Definition) {
            self.register_definition(definition).test_value();
        }

        #[track_caller]
        fn register_test_player(&mut self, player: PlayerConfig) {
            self.register_player(player).test_value();
        }

        #[track_caller]
        fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
            self.register_script_definition(id, name, script)
                .test_value();
        }

        #[track_caller]
        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
            self.spawn_object(config).test_value()
        }

        #[track_caller]
        fn test_object_index(&self, id: ObjectId) -> usize {
            self.find_object_index(id).test_value()
        }

        #[track_caller]
        fn call_test_object_function(
            &mut self,
            index: usize,
            function: &str,
            args: Vec<Value>,
        ) -> Value {
            self.call_object_function(index, function, args)
                .test_value()
        }
    }

    #[track_caller]
    fn test_snapshot(engine: &Engine, id: ObjectId) -> ObjectSnapshot {
        engine.object_snapshot(id).test_value()
    }

    fn configure_connect_action(definition: &mut Definition) {
        definition.configure_actions(
            Some("Connect".to_owned()),
            HashMap::from([(
                "Connect".to_owned(),
                ActionSpec::default().with_procedure("connect"),
            )]),
        );
    }

    fn clonk_actions() -> HashMap<String, ActionSpec> {
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Jump".to_string(), ActionSpec::for_procedure("flight"));
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        actions.insert("Push".to_string(), ActionSpec::for_procedure("push"));
        actions
    }

    fn register_clonk(engine: &mut Engine, id: &str, script: &str) {
        let mut definition = test_definition(id, id, script);
        definition.configure_actions(Some("Walk".to_string()), clonk_actions());
        definition.set_movement_profile(MovementProfile::default());
        let physical = PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            dig: 40_000,
            can_dig: 1,
            ..Default::default()
        };
        definition.set_physical(physical);
        engine.register_test_definition(definition);
    }

    fn register_builder_clonk(engine: &mut Engine, id: &str, script: &str) {
        let mut definition = test_definition(id, id, script);
        definition.configure_actions(Some("Walk".to_string()), clonk_actions());
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            dig: 40_000,
            can_dig: 1,
            can_construct: 1,
            ..Default::default()
        });
        engine.register_test_definition(definition);
    }

    fn dig_double_linekit_consumption_fixture(
        swap_contents: bool,
    ) -> (Engine, ObjectId, ObjectId, ObjectId) {
        let mut engine = Engine::new();
        let mut clonk = test_definition(
            "CLNK",
            "Clonk",
            r#"#strict 2
        local own_activations;
        protected func Activate()
        {
          own_activations++;
          return(1);
        }
        "#,
        );
        clonk.configure_actions(Some("Walk".to_owned()), clonk_actions());
        clonk.set_movement_profile(MovementProfile::default());
        clonk.set_physical(PhysicalInfo {
            can_chop: 1,
            can_construct: 1,
            ..Default::default()
        });
        engine.register_test_definition(clonk);
        engine.register_test_script_definition("LNKT", "Linekit", "#strict 2\n");
        if swap_contents {
            engine
                .register_definition(test_definition(
                    "SWAP",
                    "Swap item",
                    r#"#strict 2
                    public func Activate(object clonk)
                    {
                      CreateContents(LNKT, clonk);
                      RemoveObject();
                      return(0);
                    }
                    "#,
                ))
                .test_value();
        }
        let mut tree = test_definition("TREE", "Tree", "#strict 2\n");
        tree.set_chopable(true);
        tree.set_category(crate::CATEGORY_STATIC_BACK);
        tree.set_shape_rect(Some(crate::DefinitionRect::new(-10, -20, 20, 40)));
        engine.register_test_definition(tree);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Dig")),
        );
        engine.select_crew(1, [crew]).test_value();
        engine.set_crew_cursor(1, Some(crew)).test_value();
        let tree = engine.spawn_test_object(
            SpawnConfig::new("TREE")
                .with_position(Vector2::new(100, 100))
                .with_category(crate::CATEGORY_STATIC_BACK)
                .with_loaded(true),
        );
        let initial_contents = engine.spawn_test_object(
            SpawnConfig::new(if swap_contents { "SWAP" } else { "LNKT" }).with_container(crew),
        );
        let position = test_snapshot(&engine, crew).position;
        assert!(engine
            .at_object(position, ocf::CHOP, Some(crew))
            .is_some_and(|(_, id, object_ocf)| { id == tree && object_ocf & ocf::CHOP != 0 }));
        assert!(
            engine
                .at_object(position, ocf::LINE_CONSTRUCT, Some(crew))
                .is_none(),
            "line construction must fail while the independent Chop target remains valid"
        );
        (engine, crew, tree, initial_contents)
    }

    fn spawn_crew(engine: &mut Engine, def: &str, owner: i32) -> ObjectId {
        let crew = engine.spawn_test_object(
            SpawnConfig::new(def)
                .with_owner(owner)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk")),
        );
        engine.select_crew(owner, vec![crew]).test_value();
        engine.set_crew_cursor(owner, Some(crew)).test_value();
        crew
    }

    fn clonk_engine(script: &str) -> Engine {
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
    }

    fn register_player_crew(engine: &mut Engine) -> ObjectId {
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        spawn_crew(engine, "CLNK", 1)
    }

    fn contain_object(engine: &mut Engine, object: ObjectId, container: ObjectId) {
        engine
            .apply_object_update(object, crate::ObjectUpdate::new().with_container(container))
            .test_value();
    }

    fn set_second_picture_row(engine: &mut Engine, object: ObjectId) {
        engine
            .apply_object_update(
                object,
                crate::ObjectUpdate {
                    picture_rect: Some(crate::DefinitionRect::new(0, 76, 64, 64)),
                    ..crate::ObjectUpdate::default()
                },
            )
            .test_value();
    }

    fn contain_player_crew(
        engine: &mut Engine,
        container_definition: &str,
    ) -> (ObjectId, ObjectId) {
        let crew = register_player_crew(engine);
        let container = engine.spawn_test_object(SpawnConfig::new(container_definition));
        contain_object(engine, crew, container);
        (crew, container)
    }

    fn clonk_crew_fixture(script: &str) -> (Engine, ObjectId) {
        let mut engine = clonk_engine(script);
        let crew = register_player_crew(&mut engine);
        (engine, crew)
    }

    fn structure_definition(
        id: impl Into<String>,
        name: impl Into<String>,
        script: &str,
    ) -> Definition {
        let mut definition = test_definition(id, name, script);
        definition.set_category(crate::CATEGORY_STRUCTURE);
        definition
    }

    fn register_auto_context_structure(engine: &mut Engine, id: &str, name: &str, script: &str) {
        let mut definition = structure_definition(id, name, script);
        definition.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        definition.set_auto_context_menu(true);
        engine.register_test_definition(definition);
    }

    fn test_object(engine: &Engine, object: ObjectId) -> &crate::object::Object {
        let index = engine.test_object_index(object);
        &engine.objects[index]
    }

    fn test_object_mut(engine: &mut Engine, object: ObjectId) -> &mut crate::object::Object {
        let index = engine.test_object_index(object);
        &mut engine.objects[index]
    }

    fn object_local<'a>(engine: &'a Engine, object: ObjectId, name: &str) -> Option<&'a Value> {
        let index = engine.test_object_index(object);
        engine.objects[index].state.local_vars.get(name)
    }

    fn test_menu(engine: &Engine, object: ObjectId) -> crate::ObjectMenuState {
        engine
            .debug_object_menu(object.as_u64())
            .expect("object exists")
            .test_value()
    }

    fn open_activate_test_menu(
        engine: &mut Engine,
        crew: ObjectId,
        container: ObjectId,
    ) -> crate::ObjectMenuState {
        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_activate_menu(crew_index, container_index)
            .test_value();
        test_menu(engine, crew)
    }

    fn open_contents_test_menu(
        engine: &mut Engine,
        crew: ObjectId,
        container: ObjectId,
        identification: i32,
    ) -> crate::ObjectMenuState {
        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_container_contents_menu(crew_index, container_index, identification)
            .test_value();
        test_menu(engine, crew)
    }

    fn menu_captions(menu: &crate::ObjectMenuState) -> Vec<&str> {
        menu.items
            .iter()
            .map(|item| item.caption.as_str())
            .collect()
    }

    fn menu_item_ids(menu: &crate::ObjectMenuState) -> Vec<&str> {
        menu.items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect()
    }

    fn menu_item_counts(menu: &crate::ObjectMenuState) -> Vec<(&str, i32)> {
        menu.items
            .iter()
            .map(|item| (item.item_id.as_str(), item.count))
            .collect()
    }

    fn menu_picture_objects(menu: &crate::ObjectMenuState) -> Vec<Option<ObjectId>> {
        menu.items.iter().map(|item| item.picture_object).collect()
    }

    fn menu_pictures_are_owned(menu: &crate::ObjectMenuState) -> bool {
        menu.items
            .iter()
            .all(|item| item.picture_snapshot.is_some())
    }

    fn register_menu_image_definitions(engine: &mut Engine, ids: &[&str]) {
        for id in ids {
            engine.register_test_script_definition(id, id, "#strict 2\n");
        }
    }

    fn open_native_context(
        engine: &mut Engine,
        crew: ObjectId,
        target: ObjectId,
    ) -> crate::ObjectMenuState {
        let crew_index = engine.test_object_index(crew);
        let target_index = engine.test_object_index(target);
        engine
            .open_context_menu(crew_index, target_index, false, None)
            .test_value();
        engine
            .debug_object_menu(crew.as_u64())
            .expect("crew survives")
            .test_value()
    }

    fn line_pickup_gate_fixture(
        collection_limit: i32,
        actor_has_rock: bool,
        other_endpoint_is_kit: bool,
    ) -> (Engine, ObjectId, ObjectId, ObjectId, ObjectId) {
        line_pickup_gate_fixture_with_linekit(
            collection_limit,
            actor_has_rock,
            other_endpoint_is_kit,
            "#strict\n",
            "#strict\n",
        )
    }

    fn line_pickup_gate_fixture_with_linekit(
        collection_limit: i32,
        actor_has_rock: bool,
        other_endpoint_is_kit: bool,
        clonk_script: &str,
        linekit_script: &str,
    ) -> (Engine, ObjectId, ObjectId, ObjectId, ObjectId) {
        let mut engine = Engine::new();
        let mut clonk = test_definition("CLNK", "Clonk", clonk_script);
        clonk.configure_actions(Some("Walk".to_owned()), clonk_actions());
        clonk.set_movement_profile(MovementProfile::default());
        clonk.set_physical(PhysicalInfo {
            can_construct: 1,
            ..Default::default()
        });
        clonk.set_collection_limit(collection_limit);
        engine.register_test_definition(clonk);
        engine.register_test_script_definition("LNKT", "Linekit", linekit_script);
        if actor_has_rock {
            engine.register_test_script_definition("ROCK", "Rock", "#strict\n");
        }

        let mut structure = test_definition("POWR", "Generator", "#strict\n");
        structure.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        structure.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine.register_test_definition(structure);
        let mut endpoint = test_definition("CONS", "Consumer", "#strict\n");
        endpoint.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        endpoint.set_line_connect(crate::LINE_CONNECT_POWER_INPUT);
        engine.register_test_definition(endpoint);
        let mut line = test_definition(
            "PWRL",
            "Power line",
            "#strict\npublic func Initialize() { SetAction(\"Connect\"); return(1); }\n",
        );
        line.set_line(1);
        line.set_shape_vertices(vec![
            crate::ObjectVertex::default(),
            crate::ObjectVertex::default(),
        ]);
        configure_connect_action(&mut line);
        engine.register_test_definition(line);

        let structure = engine
            .spawn_test_object(SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)));
        let endpoint = if other_endpoint_is_kit {
            engine.spawn_test_object(SpawnConfig::new("LNKT").with_position(Vector2::new(200, 100)))
        } else {
            engine.spawn_test_object(SpawnConfig::new("CONS").with_position(Vector2::new(200, 120)))
        };
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Walk")),
        );
        if actor_has_rock {
            engine.spawn_test_object(SpawnConfig::new("ROCK").with_container(crew));
        }
        let mut connect_action = ActionState::new("Connect");
        connect_action.target = Some(structure);
        connect_action.target2 = Some(endpoint);
        let line = engine.spawn_test_object(
            SpawnConfig::new("PWRL")
                .with_owner(2)
                .with_action(connect_action),
        );
        (engine, crew, line, structure, endpoint)
    }

    fn engine_with_counted_crew_script(script: &str) -> (Engine, ObjectId, ObjectId) {
        let mut engine = Engine::with_seed(0);
        register_clonk(&mut engine, "Test", script);
        engine.register_test_player(PlayerConfig::new(0, "Test"));
        let spawn = |engine: &mut Engine| {
            engine.spawn_test_object(
                SpawnConfig::new("Test")
                    .with_owner(0)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Walk")),
            )
        };
        let first = spawn(&mut engine);
        let second = spawn(&mut engine);
        engine.select_crew(0, [first, second]).test_value();
        engine.set_crew_cursor(0, Some(first)).test_value();

        let roster_entry = |name: &str, experience: i32| crate::player_file::CrewInfo {
            id: "Test".to_string(),
            name: name.to_string(),
            core: Default::default(),
            rank_name: "Clonk".to_string(),
            experience,
            in_action: true,
            was_in_action: true,
            portraits: Default::default(),
            ..Default::default()
        };
        engine
            .crew_rosters
            .insert(0, vec![roster_entry("First", 0), roster_entry("Second", 0)]);
        engine.crew_info_order.insert(0, vec![0, 1]);
        for (object, roster_index, name, experience) in
            [(first, 0, "First", 0), (second, 1, "Second", 0)]
        {
            Rc::make_mut(&mut engine.crew_object_infos).insert(
                object,
                CrewObjectInfo {
                    definition_id: "Test".into(),
                    name: name.to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 0,
                    rank_name: "Clonk".to_string(),
                    experience,
                    participation: 1,
                    rounds: 0,
                    death_count: 0,
                    total_playing_time: 0,
                    birthday: 0,
                    age: 0,
                    in_action_time: 0,
                    extra_data: Vec::new(),
                    portraits: Default::default(),
                },
            );
            Rc::make_mut(&mut engine.crew_info_links).insert(
                object,
                CrewInfoLink {
                    player_id: 0,
                    roster_index,
                },
            );
            Rc::make_mut(&mut engine.crew_ranks).insert(object.as_u64(), 0);
        }
        (engine, first, second)
    }

    fn engine_with_counted_crew() -> (Engine, ObjectId, ObjectId) {
        engine_with_counted_crew_script("")
    }

    fn execute_player_controls(
        engine: &mut Engine,
        controls: impl IntoIterator<Item = (i32, i32)>,
    ) {
        for (command, data) in controls {
            engine.execute_player_control(0, command, data).test_value();
        }
    }

    #[test]
    fn command_success_experience_uses_shared_control_count_and_exact_modulo() {
        fn finish_native(engine: &mut Engine, actor: ObjectId, request: CommandRequest) {
            let command = request.id;
            let index = engine.test_object_index(actor);
            engine.objects[index]
                .commands
                .push_front(request)
                .test_value();
            assert!(engine.objects[index].commands.finish_front_if(command));
            engine.finish_object_command_execution(actor).test_value();
        }

        let (mut engine, first, second) = engine_with_counted_crew();
        engine.do_object_experience(first, 999);

        finish_native(&mut engine, first, CommandRequest::new(CommandId::Wait));
        assert_eq!(engine.crew_info_control_count(first), Some(0));
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 999);

        finish_native(
            &mut engine,
            first,
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(0))
                .with_ty(Some(0)),
        );
        finish_native(&mut engine, first, CommandRequest::new(CommandId::Acquire));
        assert_eq!(engine.crew_info_control_count(first), Some(3));

        let index = engine.test_object_index(first);
        engine.objects[index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Attack)
                    .with_target(Some(ObjectId::new(999_999)))
                    .with_mode(CommandMode::Base),
            )
            .test_value();
        engine.execute_object_command_now(first).test_value();
        assert_eq!(engine.crew_info_control_count(first), Some(3));
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 999);

        for _ in 0..2 {
            finish_native(
                &mut engine,
                first,
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(0))
                    .with_ty(Some(0)),
            );
        }
        assert_eq!(engine.crew_info_control_count(first), Some(5));
        let info = engine.crew_object_info(first).test_value();
        assert_eq!((info.experience, info.rank), (1_000, 1));
        assert_eq!(info.rank_name, "Ensign");

        finish_native(
            &mut engine,
            first,
            CommandRequest::new(CommandId::Build).with_target(Some(second)),
        );
        assert_eq!(engine.crew_info_control_count(first), Some(10));
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 1_001);

        finish_native(
            &mut engine,
            first,
            CommandRequest::new(CommandId::Attack).with_target(Some(second)),
        );
        assert_eq!(engine.crew_info_control_count(first), Some(25));
        let info = engine.crew_object_info(first).test_value();
        assert_eq!((info.experience, info.rank), (1_004, 1));
        let roster = &engine.crew_rosters[&0][0];
        assert_eq!((roster.experience, roster.rank), (1_004, 1));
        assert_eq!(roster.rank_name, "Ensign");

        let index = engine.test_object_index(first);
        engine.objects[index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(0))
                    .with_ty(Some(0)),
            )
            .test_value();
        engine.objects[index].commands.finish_entry_public(0, true);
        engine.finish_object_command_execution(first).test_value();
        assert_eq!(
            engine.crew_info_control_count(first),
            Some(25),
            "FnFinishCommand sets Finished without calling native Finish(true)"
        );
    }

    #[test]
    fn native_finish_awards_after_callback_detaches_or_prefinishes_command() {
        let script = r#"#strict
local native_finish_mode;

public func SetNativeFinishMode(int mode)
{
    native_finish_mode = mode;
    return true;
}

protected func OnActionJump()
{
    if (native_finish_mode == 1) SetCommand(this(), "Wait");
    if (native_finish_mode == 2) FinishCommand(this(), true, 0);
    return true;
}
"#;
        let (mut engine, detached, prefinished) = engine_with_counted_crew_script(script);

        for actor in [detached, prefinished] {
            engine.count_crew_info_control(actor, 4);
        }

        for (actor, mode) in [(detached, 1), (prefinished, 2)] {
            let index = engine.test_object_index(actor);
            assert_eq!(
                engine
                    .call_object_function(index, "SetNativeFinishMode", vec![Value::Int(mode)])
                    .expect("configure the jump callback"),
                Value::Bool(true)
            );
            let index = engine.test_object_index(actor);
            engine.objects[index]
                .commands
                .push_front(CommandRequest::new(CommandId::Jump))
                .test_value();
            engine.execute_object_command_now(actor).test_value();

            assert_eq!(engine.crew_info_control_count(actor), Some(5));
            assert_eq!(
                engine.crew_object_info(actor).unwrap().experience,
                1,
                "native Finish(true) awards after callback mode {mode}"
            );
        }

        assert_eq!(
            test_object(&engine, detached).commands.command_names(),
            ["Wait"],
            "SetCommand detached the executing Jump before its native finish tail"
        );
    }

    #[test]
    fn synchronous_execute_command_awards_before_finished_callback() {
        let script = r#"#strict
local callback_experience, callback_rank;

protected func ControlCommandFinished()
{
    callback_experience = GetObjectInfoCoreVal("Experience", "ObjectInfo");
    callback_rank = GetRank();
}

public func CompleteNative()
{
    SetCommand(this(), "Context", 0, 0, 0, this());
    return ExecuteCommand();
}

public func MarkFinishedOnly()
{
    SetCommand(this(), "MoveTo", 0, GetX(), GetY());
    FinishCommand(this(), true, 0);
    return ExecuteCommand();
}
"#;
        let (mut engine, first, _) = engine_with_counted_crew_script(script);
        engine.do_object_experience(first, 999);
        engine.count_crew_info_control(first, 4);
        let index = engine.test_object_index(first);

        assert_eq!(
            engine
                .call_object_function(index, "CompleteNative", Vec::new())
                .expect("native Context completes synchronously"),
            Value::Bool(true)
        );
        assert_eq!(engine.crew_info_control_count(first), Some(5));
        let info = engine.crew_object_info(first).test_value();
        assert_eq!((info.experience, info.rank), (1_000, 1));
        let index = engine.test_object_index(first);
        assert_eq!(
            engine.objects[index]
                .state
                .local_vars
                .get("callback_experience"),
            Some(&Value::Int(1_000))
        );
        assert_eq!(
            engine.objects[index].state.local_vars.get("callback_rank"),
            Some(&Value::Int(1))
        );

        assert_eq!(
            engine
                .call_object_function(index, "MarkFinishedOnly", Vec::new())
                .expect("script-finished MoveTo clears synchronously"),
            Value::Bool(true)
        );
        assert_eq!(
            engine.crew_info_control_count(first),
            Some(5),
            "FnFinishCommand does not call C4Command::Finish(true)"
        );
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 1_000);
    }

    #[test]
    fn player_control_count_replay_awards_the_pre_in_com_cursor() {
        let (mut engine, first, second) = engine_with_counted_crew();
        let mut controls = (0..4)
            .map(|data| (i32::from(COM_THROW), data))
            .collect::<Vec<_>>();
        controls.push((i32::from(COM_CURSOR_RIGHT), 0));
        execute_player_controls(&mut engine, controls);

        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (5, 5));
        assert_eq!(engine.crew_info_control_count(first), Some(5));
        assert_eq!(engine.crew_info_control_count(second), Some(0));
        assert_eq!(engine.crew_cursor(0), Some(second));

        let first_info = engine.crew_object_info(first).test_value();
        assert_eq!((first_info.experience, first_info.rank), (1, 0));
        assert_eq!(
            (
                engine.crew_rosters[&0][0].experience,
                engine.crew_rosters[&0][0].rank,
            ),
            (1, 0),
            "the persistent roster receives the same DoExperience result"
        );
        assert_eq!(engine.crew_object_info(second).unwrap().experience, 0);
    }

    #[test]
    fn player_control_count_release_range_uses_the_raw_signed_command() {
        let (mut engine, first, _) = engine_with_counted_crew();
        execute_player_controls(&mut engine, (17..=30).map(|command| (command, command)));
        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (0, 0));
        assert_eq!(engine.crew_info_control_count(first), Some(0));
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 0);

        execute_player_controls(
            &mut engine,
            [16, 31, 273, -1].into_iter().map(|command| (command, 0)),
        );
        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (4, 4));
        assert_eq!(engine.crew_info_control_count(first), Some(4));
    }

    #[test]
    fn player_control_count_deduplicates_the_computed_type_and_id_pair() {
        let (mut engine, first, _) = engine_with_counted_crew();
        execute_player_controls(
            &mut engine,
            [
                (i32::from(COM_THROW), 7),
                (21, 0),
                (i32::from(COM_DOWN), 10_007),
            ],
        );

        let player = engine.player(0).test_value();
        assert_eq!(
            (player.control_count(), player.action_count()),
            (2, 1),
            "the release does not count or break the equal 50,007 checksum streak"
        );
        assert_eq!(engine.crew_info_control_count(first), Some(1));
        assert_eq!(engine.crew_object_info(first).unwrap().experience, 0);
    }

    #[test]
    fn player_control_count_deduplicates_by_control_type_and_id() {
        let (mut engine, first, _) = engine_with_counted_crew();
        engine.count_player_control(0, CountedControlType::DirectCom, 50_007, 1);
        engine.count_player_control(0, CountedControlType::Command, 50_007, 1);
        engine.count_player_control(0, CountedControlType::Command, 50_007, 1);

        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (3, 2));
        assert_eq!(engine.crew_info_control_count(first), Some(2));
    }

    #[test]
    fn player_control_count_runs_at_the_packet_layer_not_inside_in_com() {
        let (mut engine, first, _) = engine_with_counted_crew();
        engine.player_in_com(0, COM_THROW, 3).test_value();
        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (0, 0));
        assert_eq!(engine.crew_info_control_count(first), Some(0));

        execute_player_controls(&mut engine, [(i32::from(COM_THROW), 3)]);
        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (1, 1));
        assert_eq!(engine.crew_info_control_count(first), Some(1));
    }

    #[test]
    fn player_command_count_uses_the_raw_five_field_checksum_once_per_packet() {
        let (mut engine, first, _) = engine_with_counted_crew();
        engine.count_player_control(0, CountedControlType::DirectCom, 961, 1);

        let packets = [
            (CommandId::Wait as i32, 20, 30, 400, 500, 1),
            (CommandId::Wait as i32, 19, 30, 401, 500, 2),
            (CommandId::Wait as i32, 18, 30, 401, 501, 3),
            (CommandId::Wait as i32, 17, 31, 401, 501, 4),
            (CommandId::MoveTo as i32, 26, 31, 401, 501, 5),
        ];
        for (command, x, y, target, target2, data) in packets {
            engine
                .execute_player_command(0, command, x, y, target, target2, data, 0)
                .test_value();
        }

        let player = engine.player(0).test_value();
        assert_eq!(
            (player.control_count(), player.action_count()),
            (6, 2),
            "five packets count once each; equal Command checksums deduplicate independently of DirectCom and Data"
        );
        assert_eq!(engine.crew_info_control_count(first), Some(2));
    }

    #[test]
    fn player_control_count_resets_for_a_reused_player_number() {
        let (mut engine, first, _) = engine_with_counted_crew();
        execute_player_controls(&mut engine, [(i32::from(COM_THROW), 3)]);
        assert_eq!(engine.crew_info_control_count(first), Some(1));

        engine.remove_player(0).test_value();
        engine.register_test_player(PlayerConfig::new(0, "Replacement"));

        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (0, 0));
        assert!(
            engine
                .crew_info_control_counts
                .keys()
                .all(|link| link.player_id != 0),
            "a new CrewInfoList cannot inherit runtime counters"
        );
    }

    /// A collector clonk + a collectible item inside it, ready for the
    /// drop→NoCollectDelay→recollect window tests.
    fn drop_window_fixture(engine: &mut Engine) -> (ObjectId, ObjectId) {
        let mut clonk = test_definition("CLNK", "Clonk", "#strict\n");
        clonk.configure_actions(Some("Walk".to_string()), clonk_actions());
        clonk.set_movement_profile(MovementProfile::default());
        clonk.set_collection_rect(Some(crate::DefinitionRect::new(-8, -16, 16, 32)));
        engine.register_test_definition(clonk);
        let mut item = test_definition("GOLD", "Gold", "#strict\n");
        item.set_collectible(true);
        engine.register_test_definition(item);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = spawn_crew(engine, "CLNK", 1);
        let item = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(crew));
        (crew, item)
    }

    fn no_collect_delay(engine: &Engine, id: ObjectId) -> i32 {
        let index = engine.test_object_index(id);
        engine.objects[index].state.no_collect_delay
    }

    #[test]
    fn cursor_script_menu_consumes_controls_before_gameplay_like_cpp() {
        // C4Player::InCom converts regular cursor-menu input before the
        // single/double machinery (C4Player.cpp:1502-1513), then
        // C4Object::DirectCom gives Menu->Control first refusal
        // (C4Object.cpp:3363-3371). Dragon Rock depends on this ordering:
        // its mandatory difficulty/type menus must complete before Up can
        // become ObjectComUp/Jump.
        let script = r#"
        local chosen;
        func OpenMenu() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("First", "Choose(1)", WIPF, this());
            AddMenuItem("Second", "Choose(2)", WIPF, this());
            return 1;
        }
        func Choose(value) { chosen = value; return 1; }
        func MenuQueryCancel() { return 1; }
        "#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        let index = engine.test_object_index(crew);
        engine.call_test_object_function(index, "OpenMenu", Vec::new());

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 0);

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 1, "Right navigates the script menu");
        assert_eq!(
            test_snapshot(&engine, crew).command_direction,
            CommandDirection::Stop,
            "menu navigation must not leak into gameplay steering"
        );
        engine
            .player_in_com(1, COM_RIGHT + COM_RELEASE_OFFSET, 0)
            .test_value();
        assert_eq!(
            test_menu(&engine, crew).selection,
            1,
            "the raw release neither navigates again nor leaks"
        );

        engine.player_in_com(1, COM_DIG, 0).test_value();
        assert!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .is_some(),
            "MenuQueryCancel may deny the soft close"
        );

        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        assert_eq!(
            test_object(&engine, crew).state.local_vars.get("chosen"),
            Some(&Value::Int(2)),
            "Enter executes the selected command"
        );
        engine
            .player_in_com(1, COM_THROW + COM_RELEASE_OFFSET, 0)
            .test_value();

        engine.player_in_com(1, COM_UP, 0).test_value();
        engine.tick_without_snapshot().test_value();
        assert_eq!(
            test_snapshot(&engine, crew).action.name,
            "Jump",
            "once the mandatory menu closes, Up reaches ObjectComUp"
        );
    }

    #[test]
    fn clear_menu_items_keeps_location_reset_when_same_count_is_readded() {
        // FnClearMenuItems passes fResetSelection=true, so C4Menu clears
        // LocationSet even if a callback re-adds the old number of rows
        // (C4Script.cpp:5149-5159; C4Menu.cpp:975-987).
        let script = r#"
        #strict 2
        func OpenMenu() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("First", "Nop()", WIPF, this());
            AddMenuItem("Second", "Nop()", WIPF, this());
            return 1;
        }
        func AppendOnly() {
            AddMenuItem("Appended", "Nop()", WIPF, this());
            return 1;
        }
        func ClearAndReadd() {
            ClearMenuItems(this());
            AddMenuItem("Replacement", "Nop()", WIPF, this());
            AddMenuItem("Replacement 2", "Nop()", WIPF, this());
            return 1;
        }
        func Nop() { return 1; }
        "#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        let index = engine.test_object_index(crew);
        engine.call_test_object_function(index, "OpenMenu", Vec::new());
        let initial_generation = test_menu(&engine, crew).location_reset_generation;

        engine.call_test_object_function(index, "AppendOnly", Vec::new());
        let appended = test_menu(&engine, crew);
        assert_eq!(appended.items.len(), 3);
        assert_eq!(
            appended.location_reset_generation, initial_generation,
            "ordinary AddMenuItem does not clear LocationSet (C4Menu.cpp:401-430)"
        );

        engine.call_test_object_function(index, "ClearAndReadd", Vec::new());
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.items.len(), 2);
        assert_eq!(
            menu.location_reset_generation,
            initial_generation.wrapping_add(1),
        );
    }

    #[test]
    fn scenario_script_menu_routes_enter_close_and_selection_callbacks_like_cpp() {
        let scenario = r#"
        #strict 2
        static menu_owner;

        func Open(obj) {
            menu_owner = obj;
            CreateMenu(WIPF, obj, 0, 0, "Scenario");
            AddMenuItem("First", "Choose(11)", WIPF, obj);
            AddMenuItem("Second", "Choose(22)", WIPF, obj);
            return 1;
        }

        func Choose(value) {
            SetWealth(1, value);
            return 1;
        }

        func OnMenuSelection(selection, parent) {
            if (selection == 1 && parent == menu_owner) SetWealth(1, 101);
            return 1;
        }

        func MenuQueryCancel(selection, parent) {
            if (selection == 1 && parent == menu_owner) {
                SetWealth(1, 201);
                return 1;
            }
            return 0;
        }
        "#;
        let (mut engine, crew) = clonk_crew_fixture("#strict 2\n");
        engine
            .install_scenario_script_with_convention("Scenario", scenario, true)
            .test_value();
        engine
            .call_scenario_script_function("Open", vec![compat::object_reference_value(crew)])
            .test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 0);
        assert_eq!(
            menu.command_object, None,
            "scenario scope selects CB_Scenario"
        );
        assert!(menu.scenario_callbacks);

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        assert_eq!(
            engine.player(1).expect("player").wealth(),
            101,
            "OnMenuSelection receives the live selection and parent object"
        );

        engine.player_in_com(1, COM_DIG, 0).test_value();
        assert!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .is_some(),
            "scenario MenuQueryCancel may deny the soft close"
        );
        assert_eq!(engine.player(1).expect("player").wealth(), 201);

        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(
            engine.debug_object_menu(crew.as_u64()),
            Some(None),
            "non-permanent menu closes before its command executes"
        );
        assert_eq!(
            engine.player(1).expect("player").wealth(),
            22,
            "the selected command executes in scenario scope"
        );
    }

    #[test]
    fn cleared_object_menu_callback_does_not_become_scenario_callback() {
        let object_script = r#"
        #strict 2
        func OpenMenu(command) {
            CreateMenu(WIPF, this(), command, 0, "Object");
            AddMenuItem("First", "Choose(11)", WIPF, this());
            AddMenuItem("Second", "Choose(22)", WIPF, this());
            return 1;
        }
        func Choose(value) { SetWealth(1, value); return 1; }
        func OnMenuSelection() { SetWealth(1, 301); return 1; }
        func MenuQueryCancel() { SetWealth(1, 302); return 1; }
        "#;
        let scenario = r#"
        #strict 2
        func Choose(value) { SetWealth(1, value); return 1; }
        func OnMenuSelection() { SetWealth(1, 101); return 1; }
        func MenuQueryCancel() { SetWealth(1, 201); return 1; }
        "#;
        let (mut engine, crew) = clonk_crew_fixture(object_script);
        engine
            .install_scenario_script_with_convention("Scenario", scenario, true)
            .test_value();

        let open_menu = |engine: &mut Engine, command_object: ObjectId| {
            let crew_index = engine.test_object_index(crew);
            engine.call_test_object_function(
                crew_index,
                "OpenMenu",
                vec![compat::object_reference_value(command_object)],
            );
        };

        let command_object = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        open_menu(&mut engine, command_object);
        engine.assign_object_removal(command_object).test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.command_object, None, "the object pointer is cleared");
        assert!(
            !menu.scenario_callbacks,
            "ClearPointers does not change the captured CB_Object type"
        );

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        assert_eq!(
            engine.player(1).expect("player").wealth(),
            0,
            "selection does not fall through to the scenario callback"
        );
        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        assert_eq!(
            engine.player(1).expect("player").wealth(),
            0,
            "Enter does not run the copied command in scenario scope"
        );

        let second_command_object = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        open_menu(&mut engine, second_command_object);
        engine
            .assign_object_removal(second_command_object)
            .test_value();
        engine.player_in_com(1, COM_DIG, 0).test_value();
        assert_eq!(
            engine.debug_object_menu(crew.as_u64()),
            Some(None),
            "soft close is not denied by the scenario callback"
        );
        assert_eq!(engine.player(1).expect("player").wealth(), 0);
    }

    #[test]
    fn menu_show_text_reveals_every_progressive_row_like_cpp() {
        // C4Menu::Control(COM_MenuShowText) calls SetTextProgress(-1),
        // revealing all rows without activating a command (C4Menu.cpp:
        // 477-480). This command is already converted and synchronized.
        let script = r#"
        func OpenMenu() {
            CreateMenu(CLNK, this(), this(), 0, "", 0, 3);
            AddMenuItem("First", "", NONE, this());
            AddMenuItem("Continue", "Choose", CLNK, this());
            AddMenuItem("Last", "", NONE, this());
            return SetMenuTextProgress(0, this());
        }
        "#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        let index = engine.test_object_index(crew);
        engine.call_test_object_function(index, "OpenMenu", Vec::new());
        assert!(test_menu(&engine, crew).text_progressing);

        engine.player_in_com(1, COM_MENU_SHOW_TEXT, 0).test_value();
        let menu = test_menu(&engine, crew);
        assert!(!menu.text_progressing);
        assert!(menu
            .items
            .iter()
            .all(|item| item.text_display_progress == -1));
    }

    #[test]
    fn empty_script_menu_ignores_explicit_select_like_cpp() {
        // C4Menu::Control guards COM_MenuSelect with ItemCount before
        // SetSelection and its callback (C4Menu.cpp:474-476).
        let script = r#"
        local selection_calls;
        func OpenMenu() {
            selection_calls = 0;
            CreateMenu(WIPF, this(), this(), 0, "Empty");
            return 1;
        }
        func OnMenuSelection() { selection_calls++; return 1; }
        "#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        let index = engine.test_object_index(crew);
        engine.call_test_object_function(index, "OpenMenu", Vec::new());

        engine.player_in_com(1, COM_MENU_SELECT, 0).test_value();
        assert_eq!(
            test_object(&engine, crew)
                .state
                .local_vars
                .get("selection_calls"),
            Some(&Value::Nil),
            "an empty menu must not run OnMenuSelection"
        );
    }

    #[test]
    fn drop_command_arms_no_collect_delay_and_clears_collection_ocf() {
        // ObjectComDrop (C4ObjectCom.cpp:668-671): after the item exits,
        // `cObj->NoCollectDelay = 2` and the immediate SetOCF drop the
        // dropper's OCF_Collection bit (SetOCF, C4Object.cpp:598-600).
        let mut engine = Engine::new();
        let (crew, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .test_value();
        engine.tick_without_snapshot().test_value();
        assert_eq!(
            test_object(&engine, item).state.container,
            None,
            "the drop exited the item"
        );
        assert_eq!(
            no_collect_delay(&engine, crew),
            2,
            "ObjectComDrop arms NoCollectDelay = 2 (C4ObjectCom.cpp:669)"
        );
        assert_eq!(
            test_object(&engine, crew).state.ocf & ocf::COLLECTION,
            0,
            "the post-drop SetOCF clears OCF_Collection (C4ObjectCom.cpp:671)"
        );
    }

    #[test]
    fn object_com_drop_suppresses_departure_after_ejection_deletes_item() {
        // C4Object::Call checks raw Status before each callback. Ejection
        // may delete the dropped item, in which case its later Departure
        // call is a silent miss.
        let actor_script = r#"#strict 2
local callback_order;
protected func Ejection(item)
{
  callback_order = callback_order * 10 + 1;
  RemoveObject(item);
  return(1);
}
public func NoteDeparture()
{
  callback_order = callback_order * 10 + 2;
  return(1);
}
"#;
        let item_script = r#"#strict 2
protected func Departure(parent)
{
  parent->NoteDeparture();
  return(1);
}
"#;
        let mut actor = test_definition("DDAC", "Deleting dropper", actor_script);
        actor.set_c4_callback_convention(true);
        actor.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            )]),
        );
        let mut item = test_definition("DDIT", "Deleted item", item_script);
        item.set_c4_callback_convention(true);
        let mut engine = Engine::new();
        engine.register_test_definition(actor);
        engine.register_test_definition(item);
        let actor_id = engine.spawn_test_object(
            SpawnConfig::new("DDAC")
                .with_command_direction(CommandDirection::Right)
                .with_action(ActionState::new("Walk")),
        );
        let item_id = engine.spawn_test_object(SpawnConfig::new("DDIT").with_container(actor_id));

        assert!(engine
            .object_com_drop(actor_id, item_id)
            .expect("drop succeeds"));

        let actor_index = engine.test_object_index(actor_id);
        assert_eq!(
            object_local(&engine, actor_id, "callback_order"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            test_object(&engine, item_id).state.status,
            crate::ObjectStatus::Deleted
        );
    }

    #[test]
    fn put_away_unused_object_drops_immediately_when_push_put_fails() {
        let mut engine = Engine::new();
        let (crew, item) = drop_window_fixture(&mut engine);
        let target = test_definition("PAUT", "No-put target", "#strict 2\n");
        engine.register_test_definition(target);
        let target = engine.spawn_test_object(SpawnConfig::new("PAUT"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action = ActionState::new("Push");
        engine.objects[crew_index].state.action.target = Some(target);

        assert!(engine
            .put_away_unused_object(crew, None)
            .expect("put-away succeeds"));

        assert_eq!(test_object(&engine, item).state.container, None);
        let crew_index = engine.test_object_index(crew);
        assert_eq!(engine.objects[crew_index].state.no_collect_delay, 2);
        assert_eq!(engine.objects[crew_index].state.action.name, "Walk");
        assert!(
            engine.objects[crew_index]
                .commands
                .snapshot()
                .command_names()
                .is_empty(),
            "the uncontained fallback is a live drop, not a queued Drop command"
        );
    }

    #[test]
    fn set_command_control_path_decrements_no_collect_delay() {
        // C4Object::SetCommand decrements NoCollectDelay at entry
        // (C4Object.cpp:3941-3942). A single COM_Up press in WALK counts
        // down twice: once in DirectCom (:3359-3362) and once in the Jump
        // command's SetCommand (ObjectComUp -> PlayerObjectCommand ->
        // ObjectCommand2Obj Set mode, C4Player.cpp:1450).
        let mut engine = Engine::new();
        let (crew, _) = drop_window_fixture(&mut engine);
        test_object_mut(&mut engine, crew).state.no_collect_delay = 2;

        engine.player_in_com(1, COM_UP, 0).test_value();
        assert_eq!(
            no_collect_delay(&engine, crew),
            0,
            "DirectCom + SetCommand each count the delay down once"
        );
    }

    #[test]
    fn script_set_command_decrements_no_collect_delay() {
        // FnSetCommand routes through C4Object::SetCommand
        // (C4Script.cpp:866), whose entry decrement (C4Object.cpp:3941-3942)
        // must also fire for script-issued commands.
        let script = r#"
#strict
public func DoWait() { SetCommand(this(), "Wait"); return(1); }
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        let index = engine.test_object_index(crew);
        engine.objects[index].state.no_collect_delay = 2;

        engine.call_test_object_function(index, "DoWait", Vec::new());
        assert_eq!(
            no_collect_delay(&engine, crew),
            1,
            "script SetCommand counts the delay down once (C4Object.cpp:3941)"
        );
    }

    #[test]
    fn drop_window_closes_after_a_control_and_the_item_is_recollected() {
        // The full C++ window: drop arms NoCollectDelay = 2
        // (C4ObjectCom.cpp:669); the next plain control counts it down in
        // DirectCom (C4Object.cpp:3359-3362) AND in the resulting Set-mode
        // command's SetCommand (:3941-3942) — after ONE control the
        // collector's OCF_Collection returns and the Tick3 cross check
        // recollects the item (C4GameObjects.cpp:185-194).
        let mut engine = Engine::new();
        let (crew, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .test_value();
        for _ in 0..6 {
            engine.tick_without_snapshot().test_value();
        }
        let item_index = engine.test_object_index(item);
        assert_eq!(
            engine.objects[item_index].state.container, None,
            "armed delay keeps the item on the ground"
        );

        engine.player_in_com(1, COM_UP, 0).test_value();
        for _ in 0..3 {
            engine.tick_without_snapshot().test_value();
        }
        assert_eq!(
            test_object(&engine, item).state.container,
            Some(crew),
            "one control closes the window and the cross check recollects"
        );
    }

    #[test]
    fn dropped_item_is_not_recollected_while_the_delay_is_armed() {
        // While NoCollectDelay > 0 the dropper never regains OCF_Collection
        // (SetOCF, C4Object.cpp:598), so the reverse-pass cross check
        // (C4GameObjects.cpp:185-194) leaves the dropped item alone across
        // any number of Tick3 frames.
        let mut engine = Engine::new();
        let (_, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .test_value();
        for _ in 0..9 {
            engine.tick_without_snapshot().test_value();
        }
        assert_eq!(
            test_object(&engine, item).state.container,
            None,
            "no control was issued, so the delay never counted down and the \
             item stays on the ground"
        );
    }

    #[test]
    fn com_name_matches_cpp_comname_table() {
        // ComName (C4ObjectCom.cpp:800-852).
        assert_eq!(com_name_raw(COM_LEFT), "Left");
        assert_eq!(com_name_raw(COM_LEFT | COM_SINGLE), "LeftSingle");
        assert_eq!(com_name_raw(COM_LEFT | COM_DOUBLE), "LeftDouble");
        assert_eq!(com_name_raw(COM_LEFT + COM_RELEASE_OFFSET), "LeftReleased");
        assert_eq!(com_name_raw(COM_DIG | COM_SINGLE), "DigSingle");
        assert_eq!(com_name_raw(COM_THROW | COM_DOUBLE), "ThrowDouble");
        assert_eq!(com_name_raw(COM_CURSOR_TOGGLE), "CursorToggle");
        assert_eq!(com_name_raw(0), "Undefined");
        assert_eq!(com_name_raw(COM_DIG | COM_SINGLE | COM_DOUBLE), "Undefined");
    }

    #[test]
    fn coms_to_com_dir_matches_cpp_table() {
        // Coms2ComDir (C4ObjectCom.cpp:903-920): only the eight listed
        // combinations map, everything else is COMD_Stop.
        assert_eq!(coms_to_com_dir(1 << COM_UP), CommandDirection::Up);
        assert_eq!(
            coms_to_com_dir((1 << COM_UP) | (1 << COM_RIGHT)),
            CommandDirection::UpRight
        );
        assert_eq!(coms_to_com_dir(1 << COM_LEFT), CommandDirection::Left);
        // Left+Right+Up is not a listed combination: stop, not up.
        assert_eq!(
            coms_to_com_dir((1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_UP)),
            CommandDirection::Stop
        );
        // Non-direction bits are masked off.
        assert_eq!(
            coms_to_com_dir((1 << COM_DIG) | (1 << COM_RIGHT)),
            CommandDirection::Right
        );
    }

    #[test]
    fn directional_control_left_script_override_consumes_the_com() {
        // CallControl runs for EVERY com (C4Object.cpp:3385-3389): a truthy
        // ControlLeft keeps the per-procedure fallback from running.
        let script = r#"
#strict
protected func ControlLeft() { return(1); }
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);

        engine.player_in_com(1, COM_LEFT, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Stop,
            "a handled ControlLeft must not reach ObjectComMovement"
        );
    }

    #[test]
    fn walk_left_falls_back_to_object_com_movement() {
        // DFA_WALK COM_Left → ObjectComMovement(COMD_Left) with the direct
        // turnaround (C4Object.cpp:3411; C4ObjectCom.cpp:220-235).
        let script = r#"
#strict
protected func ControlLeft() { return(0); }
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);

        engine.player_in_com(1, COM_LEFT, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.command_direction, CommandDirection::Left);
        assert_eq!(
            snapshot.direction,
            Direction::Left,
            "standing turnaround flips the facing (C4ObjectCom.cpp:226-231)"
        );
    }

    #[test]
    fn old_pushed_target_receives_classic_control_after_clonk_fallback() {
        // Before 4.9.5 pushed targets receive ControlLeft only after the
        // Clonk's DFA_PUSH fallback has moved it (src/C4Object.cpp:3520-3568).
        // The callback return value cannot consume that earlier fallback.
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut lorry = test_definition("LORY", "Lorry", vehicle);
        lorry.set_version([4, 9, 4, 9, 0]);
        engine.register_test_definition(lorry);
        let crew = register_player_crew(&mut engine);
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine.player_in_com(1, COM_LEFT, 0).test_value();

        assert_eq!(
            test_snapshot(&engine, crew).command_direction,
            CommandDirection::Left,
            "the old target's truthy late callback cannot consume movement"
        );
        assert_eq!(
            test_snapshot(&engine, lorry).damage,
            1,
            "the old target still receives ControlLeft after movement"
        );
    }

    #[test]
    fn pushed_no_push_enter_vehicle_straightens_instead_of_entering() {
        // ObjectComEnter checks the pushed target's raw signed NoPushEnter
        // before looking for an entrance (C4ObjectCom.cpp:316-332). A false
        // result leaves DFA_PUSH/COM_Up to straighten the pusher upward
        // (C4Object.cpp:3544-3550).
        for control_style in [false, true] {
            for no_push_enter in [0, 1, -2] {
                let mut engine = Engine::new();
                register_clonk(
                    &mut engine,
                    "CLNK",
                    r#"#strict
public func ReadNoPushEnter()
{
  return GetDefCoreVal("NoPushEnter", "DefCore", LORY);
}
"#,
                );
                let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
                lorry.set_no_push_enter(no_push_enter);
                engine.register_test_definition(lorry);
                let mut entrance = structure_definition("HUTX", "Hut", "#strict\n");
                entrance.set_shape_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
                entrance.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
                engine.register_test_definition(entrance);
                engine.register_test_player(PlayerConfig::new(1, "Test"));
                engine
                    .players
                    .get_mut(&1)
                    .test_value()
                    .control
                    .control_style = control_style;
                let crew = spawn_crew(&mut engine, "CLNK", 1);
                let lorry = engine.spawn_test_object(
                    SpawnConfig::new("LORY").with_position(Vector2::new(100, 100)),
                );
                let entrance = engine.spawn_test_object(
                    SpawnConfig::new("HUTX")
                        .with_position(Vector2::new(100, 100))
                        .with_entrance_status(true)
                        .with_loaded(true),
                );
                assert!(engine
                    .at_object(Vector2::new(100, 100), ocf::ENTRANCE, Some(lorry))
                    .is_some_and(|(_, object, object_ocf)| {
                        object == entrance && object_ocf & ocf::ENTRANCE != 0
                    }));
                let crew_index = engine.test_object_index(crew);
                engine.objects[crew_index].state.action.name = "Push".to_string();
                engine.objects[crew_index].state.action.target = Some(lorry);

                assert_eq!(
                    engine
                        .call_object_function(crew_index, "ReadNoPushEnter", Vec::new())
                        .expect("GetDefCoreVal succeeds"),
                    Value::Int(no_push_enter),
                    "GetDefCoreVal preserves the raw signed field"
                );
                engine.player_in_com(1, COM_UP, 0).test_value();

                let crew = test_snapshot(&engine, crew);
                let lorry = test_snapshot(&engine, lorry);
                if no_push_enter == 0 {
                    assert_eq!(crew.command_direction, CommandDirection::Stop);
                    let commands = lorry.command_stack.command_views();
                    assert_eq!(commands.len(), 1);
                    assert_eq!(commands[0].name, "Enter");
                    assert_eq!(commands[0].target, Some(entrance));
                } else {
                    assert_eq!(crew.command_direction, CommandDirection::Up);
                    assert!(lorry.command_stack.command_names().is_empty());
                }
            }
        }
    }

    #[test]
    fn version_4_9_5_pushed_target_consumes_classic_and_autostop_fallbacks() {
        // At 4.9.5 the target callback moves before both DFA_PUSH fallback
        // switches, and a truthy return consumes the control
        // (src/C4Object.cpp:3520-3568,3682-3738).
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        for control_style in [false, true] {
            let mut engine = clonk_engine("#strict\n");
            let mut lorry = test_definition("LORY", "Lorry", vehicle);
            lorry.set_version([4, 9, 5, 0, 0]);
            engine.register_test_definition(lorry);
            engine.register_test_player(PlayerConfig::new(1, "Test"));
            engine
                .players
                .get_mut(&1)
                .test_value()
                .control
                .control_style = control_style;
            let crew = spawn_crew(&mut engine, "CLNK", 1);
            let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
            let crew_index = engine.test_object_index(crew);
            engine.objects[crew_index].state.action.name = "Push".to_string();
            engine.objects[crew_index].state.action.target = Some(lorry);

            engine.player_in_com(1, COM_LEFT, 0).test_value();

            assert_eq!(
                test_snapshot(&engine, crew).command_direction,
                CommandDirection::Stop,
                "the modern target consumes the control for style={control_style}"
            );
            assert_eq!(test_snapshot(&engine, lorry).damage, 1);
        }
    }

    #[test]
    fn walk_up_without_entrance_queues_a_jump_command() {
        // DFA_WALK COM_Up → ObjectComUp → PlayerObjectCommand(C4CMD_Jump)
        // (C4Object.cpp:3414; C4ObjectCom.cpp:335-351).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");

        engine.player_in_com(1, COM_UP, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_stack.command_names(),
            vec!["Jump".to_string()],
            "COM_Up in WALK issues the jump command"
        );
    }

    #[test]
    fn queued_jump_runs_live_on_action_jump_before_hardcoded_launch() {
        // C4Command::Jump calls live ObjectComJump (C4Command.cpp:1056-1067),
        // whose ObjectActionJump first calls the object-owned fail-safe hook
        // OnActionJump(xdir*100, ydir*100, true). A truthy result suppresses
        // the hardcoded Jump action and velocity assignment
        // (C4ObjectCom.cpp:48-61,280-307).
        let script = r#"
#strict
local jump_calls, jump_xdir, jump_ydir, jump_by_com;
protected func OnActionJump(int xdir, int ydir, bool by_com)
{
    jump_calls++;
    jump_xdir = xdir;
    jump_ydir = ydir;
    jump_by_com = by_com;
    return true;
}
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);

        engine.player_in_com(1, COM_UP, 0).test_value();
        engine.tick_without_snapshot().test_value();

        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.velocity, Vector2::ZERO);
        assert!(snapshot.command_stack.command_names().is_empty());
        let locals = &test_object(&engine, crew).state.local_vars;
        assert_eq!(locals.get("jump_calls"), Some(&Value::Int(1)));
        assert_eq!(locals.get("jump_xdir"), Some(&Value::Int(-196)));
        assert_eq!(locals.get("jump_ydir"), Some(&Value::Int(-400)));
        assert_eq!(locals.get("jump_by_com"), Some(&Value::Bool(true)));
    }

    #[test]
    fn queued_jump_honors_no_other_action_selected_by_false_hook() {
        // ObjectActionJump uses ordinary SetActionByName("Jump"), not a
        // forced transition. A false OnActionJump may therefore select a
        // NoOtherAction action that rejects the hardcoded jump
        // (C4ObjectCom.cpp:48-61; C4Object.cpp:4111-4115).
        let script = r#"
#strict
protected func OnActionJump()
{
    SetAction("Locked");
    return false;
}
"#;
        let mut engine = Engine::new();
        let mut definition = test_definition("CLNK", "CLNK", script);
        let mut actions = clonk_actions();
        actions.insert(
            "Locked".to_string(),
            ActionSpec::default()
                .with_procedure("walk")
                .with_no_other_action(true),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            ..Default::default()
        });
        engine.register_test_definition(definition);
        let crew = register_player_crew(&mut engine);

        engine.player_in_com(1, COM_UP, 0).test_value();
        engine.tick_without_snapshot().test_value();

        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.action.name, "Locked");
        assert_eq!(snapshot.velocity, Vector2::ZERO);
        assert!(snapshot.command_stack.command_names().is_empty());
    }

    fn no_other_action_object() -> (Engine, ObjectId) {
        let mut engine = Engine::new();
        let mut definition = test_definition("LOCK", "Locked actor", "#strict\n");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Dead".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_no_other_action(true),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );
        engine.register_test_definition(definition);
        let object = engine.spawn_test_object(
            SpawnConfig::new("LOCK")
                .with_action(ActionState::new("Dead"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right)
                .with_fixed_velocity(FixedVec2::new(itofix(3), itofix(-4))),
        );
        (engine, object)
    }

    #[test]
    fn object_com_stop_rejects_no_other_action_and_only_stops_command_direction() {
        let (mut engine, object) = no_other_action_object();
        let index = engine.test_object_index(object);
        let action_before = engine.objects[index].state.action.clone();
        let velocity_before = engine.objects[index].fixed_velocity;

        assert!(!engine.object_com_stop(index).expect("ObjectComStop runs"));

        let index = engine.test_object_index(object);
        assert_eq!(engine.objects[index].state.action, action_before);
        assert_eq!(engine.objects[index].fixed_velocity, velocity_before);
        assert_eq!(
            engine.objects[index].state.command_direction,
            CommandDirection::Stop,
            "ObjectActionStand writes ComDir before its rejected Walk transition"
        );
    }

    /// `ObjectActionTumble` sets the action and then calls
    /// `cObj->SetDir(dir)` (C4ObjectCom.cpp:74-80), so the facing change runs
    /// the **new** action's TurnAction through SetActionByName
    /// (C4Object.cpp:4243-4248). Writing the facing directly skipped it — the
    /// same mistake clonk-org/clonk-rs#1124 fixed on the com path, in four
    /// more places (clonk-org/clonk-rs#1130).
    #[test]
    fn object_action_tumble_runs_the_turn_action_like_set_dir() {
        let mut engine = Engine::new();
        let mut definition = test_definition("TUMB", "Turning tumbler", "#strict\n");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_directions(2),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_directions(2)
                        .with_turn_action("Turn"),
                ),
                ("Turn".to_string(), ActionSpec::default().with_directions(2)),
            ]),
        );
        engine.register_test_definition(definition);
        let object = engine.spawn_test_object(
            SpawnConfig::new("TUMB")
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Left),
        );

        let index = engine.test_object_index(object);
        let definition_id = engine.objects[index].definition_id.clone();
        assert!(engine
            .object_action_tumble(
                index,
                &definition_id,
                Direction::Right,
                itofix(2),
                itofix(-1)
            )
            .expect("tumble applies"));

        let index = engine.test_object_index(object);
        assert_eq!(engine.objects[index].state.direction, Direction::Right);
        assert_eq!(
            engine.objects[index].state.action.name.as_str(),
            "Turn",
            "SetDir runs the new action's TurnAction on a facing change"
        );
    }

    #[test]
    fn object_action_tumble_rejects_dead_no_other_action() {
        let (mut engine, object) = no_other_action_object();
        let index = engine.test_object_index(object);
        let action_before = engine.objects[index].state.action.clone();
        let direction_before = engine.objects[index].state.direction;
        let velocity_before = engine.objects[index].fixed_velocity;
        let definition_id = engine.objects[index].definition_id.clone();

        assert!(!engine
            .object_action_tumble(
                index,
                &definition_id,
                Direction::Left,
                itofix(9),
                itofix(-8),
            )
            .expect("ObjectActionTumble runs"));

        let index = engine.test_object_index(object);
        assert_eq!(engine.objects[index].state.action, action_before);
        assert_eq!(engine.objects[index].state.direction, direction_before);
        assert_eq!(engine.objects[index].fixed_velocity, velocity_before);
    }

    #[test]
    fn object_com_jump_clears_script_visible_bottom_attachment() {
        // ObjectActionJump clears Action.t_attach's bottom bit immediately,
        // before C4Command::Finish and ControlCommandFinished
        // (C4ObjectCom.cpp:54-61; C4Object.cpp:3997-4008).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");
        let index = engine.test_object_index(crew);
        engine.objects[index].state.t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;
        engine.objects[index].frame_t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;

        engine.execute_jump_command(crew, 0).test_value();

        let index = engine.test_object_index(crew);
        assert_eq!(engine.objects[index].state.t_attach, crate::CNAT_LEFT);
        assert_eq!(engine.objects[index].frame_t_attach, crate::CNAT_LEFT);
    }

    #[test]
    fn script_native_jump_applies_mobile_and_bottom_unstick() {
        // FnJump delegates synchronously to ObjectComJump, whose regular
        // fallback sets Mobile and clears CNAT_Bottom after installing Jump
        // (C4Script.cpp:358-363; C4ObjectCom.cpp:48-61,280-307).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict\nfunc Probe() { return Jump(); }\n",
        );
        let crew = register_player_crew(&mut engine);
        let index = engine.test_object_index(crew);
        engine.objects[index].state.t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;
        engine.objects[index].frame_t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;

        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe calls native Jump"),
            Value::Bool(true)
        );

        let index = engine.test_object_index(crew);
        assert_eq!(engine.objects[index].state.action.name, "Jump");
        assert!(engine.objects[index].state.mobile);
        assert_eq!(engine.objects[index].state.t_attach, crate::CNAT_LEFT);
        assert_eq!(engine.objects[index].frame_t_attach, crate::CNAT_LEFT);
    }

    #[test]
    fn queued_jump_target_direction_obeys_current_action_direction_count() {
        // C4Command::Jump targets through C4Object::SetDir. Directions=1
        // rejects DIR_Right, even though Tx lies to the object's right
        // (C4Command.cpp:1058-1063; C4Object.cpp:4235-4253).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");
        let index = engine.test_object_index(crew);
        let target_x = engine.objects[index].state.position.x + 10;
        engine.objects[index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Jump).with_tx(Some(target_x)),
        )]);

        engine.tick_without_snapshot().test_value();

        assert_eq!(test_snapshot(&engine, crew).direction, Direction::Left);
    }

    #[test]
    fn dig_key_press_starts_digging_after_the_single_timeout() {
        // Classic dig: press COM_Dig, nothing happens (only ControlDig).
        // After C4DoubleClick frames C4Player::Execute flushes
        // COM_Dig|COM_Single (C4Player.cpp:1215-1229) whose WALK fallback is
        // ObjectComDig + the diagonal ComDir (C4Object.cpp:3416-3421).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");

        engine.player_in_com(1, COM_DIG, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.action.name, "Walk", "no dig before the timeout");

        for _ in 0..=C4_DOUBLE_CLICK {
            engine.tick_without_snapshot().test_value();
        }
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.action.name, "Dig");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::DownLeft,
            "digging aims down toward the facing - the spawn faces DIR_Left \
             like C++ (C4Object.cpp:3419)"
        );
    }

    #[test]
    fn object_com_dig_failure_emits_no_dig_object_message() {
        fn run_failure_case(can_dig: i32, reject_action: bool, name: &str) {
            let mut engine = Engine::new();
            engine.set_object_no_dig_resource_string("%s kann|nicht graben.");
            let mut definition = test_definition("CLNK", "Clonk", "#strict\n");
            let walk = ActionSpec::default()
                .with_procedure("walk")
                .with_no_other_action(reject_action);
            definition.configure_actions(
                Some("Walk".to_string()),
                HashMap::from([
                    ("Walk".to_string(), walk),
                    (
                        "Dig".to_string(),
                        ActionSpec::default().with_procedure("dig"),
                    ),
                ]),
            );
            definition.set_physical(PhysicalInfo {
                can_dig,
                ..Default::default()
            });
            engine.register_test_definition(definition);
            let actor = engine.spawn_test_object(
                SpawnConfig::new("CLNK")
                    .with_action(ActionState::new("Walk"))
                    .with_custom_name(name),
            );
            let index = engine.test_object_index(actor);
            let action_before = engine.objects[index].state.action.clone();

            assert!(!engine.object_com_dig(index).expect("ObjectComDig runs"));

            let index = engine.test_object_index(actor);
            assert_eq!(engine.objects[index].state.action, action_before);
            let messages = engine.messages.snapshot();
            assert_eq!(messages.len(), 1, "each failure emits exactly one message");
            let message = &messages[0];
            assert_eq!(message.kind, message::MessageKind::Target);
            assert_eq!(message.target, Some(actor));
            assert_eq!(message.player, None);
            assert_eq!(message.offset, Vector2::ZERO);
            assert_eq!(message.color, 0xffff_ffff);
            assert_eq!(message.flags, 0);
            assert_eq!(
                message.lines,
                vec![format!("{name} kann"), "nicht graben.".to_string()]
            );
        }

        run_failure_case(0, false, "Nichtgräber");
        run_failure_case(1, true, "Gesperrt");
    }

    #[test]
    fn queued_dig_routes_action_rejection_through_object_com_dig() {
        let mut engine = Engine::new();
        engine.set_object_no_dig_resource_string("%s cannot dig.");
        let mut definition = test_definition("CLNK", "Clonk", "#strict\n");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("walk")
                        .with_no_other_action(true),
                ),
                (
                    "Dig".to_string(),
                    ActionSpec::default().with_procedure("dig"),
                ),
            ]),
        );
        definition.set_physical(PhysicalInfo {
            can_dig: 1,
            ..Default::default()
        });
        engine.register_test_definition(definition);
        let actor = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_action(ActionState::new("Walk"))
                .with_custom_name("Queue"),
        );
        test_object_mut(&mut engine, actor).apply_command_operations([
            CommandOperation::PushFront(
                CommandRequest::new(CommandId::Dig)
                    .with_tx(Some(0))
                    .with_ty(Some(100)),
            ),
        ]);

        engine.tick_without_snapshot().test_value();

        let snapshot = test_snapshot(&engine, actor);
        assert_eq!(snapshot.action.name, "Walk");
        assert!(snapshot.command_stack.command_names().is_empty());
        let messages = engine.messages.snapshot();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].target, Some(actor));
        assert_eq!(messages[0].lines, vec!["Queue cannot dig."]);
    }

    #[test]
    fn queued_dig_applies_post_helper_data_and_steering_in_the_same_execute() {
        let mut engine = clonk_engine("#strict\n");
        let actor = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_action(ActionState::new("Walk"))
                .with_position(Vector2::new(0, 0)),
        );
        test_object_mut(&mut engine, actor).apply_command_operations([
            CommandOperation::PushFront(
                CommandRequest::new(CommandId::Dig)
                    .with_tx(Some(0))
                    .with_ty(Some(100))
                    .with_data(CommandData::Integer(1)),
            ),
        ]);

        engine.tick_without_snapshot().test_value();

        let snapshot = test_snapshot(&engine, actor);
        assert_eq!(snapshot.action.name, "Dig");
        assert_eq!(snapshot.action.data, 1);
        assert_eq!(snapshot.command_direction, CommandDirection::DownLeft);
        assert!(engine.messages.snapshot().is_empty());
    }

    #[test]
    fn dig_pressed_twice_activates_contents_via_dig_double() {
        // Two dig presses inside C4DoubleClick become COM_Dig_D
        // (C4Player::InCom, C4Player.cpp:1532-1533) → ObjectComDigDouble
        // activates the first contents object (C4ObjectCom.cpp:537-539).
        let clonk = r#"
#strict
"#;
        let scroll = r#"
#strict
public func Activate(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = clonk_engine(clonk);
        let scroll_def = test_definition("SCRL", "Scroll", scroll);
        engine.register_test_definition(scroll_def);
        let crew = register_player_crew(&mut engine);
        let item = engine.spawn_test_object(SpawnConfig::new("SCRL").with_container(crew));

        engine.player_in_com(1, COM_DIG, 0).test_value();
        engine.player_in_com(1, COM_DIG, 0).test_value();
        let snapshot = test_snapshot(&engine, item);
        assert_eq!(
            snapshot.damage, 1,
            "the contents object's Activate ran (C4ObjectCom.cpp:537-539)"
        );
    }

    #[test]
    fn dig_double_linekit_failure_never_falls_through_to_chop_or_own_activate() {
        // The carried LNKT branch returns unconditionally after
        // ObjectComLineConstruction, even when no structure can start a line
        // and the helper returns false (C4ObjectCom.cpp:542-547).
        let (mut engine, crew, tree, linekit) = dig_double_linekit_consumption_fixture(false);
        let crew_index = engine.test_object_index(crew);

        engine.object_com_dig_double(crew_index).test_value();

        let crew_index = engine.test_object_index(crew);
        let crew_state = &engine.objects[crew_index];
        assert_eq!(crew_state.state.action.name, "Walk");
        assert!(crew_state.state.contents.contains(&linekit));
        assert!(crew_state.commands.is_empty(), "no Chop command queued");
        assert!(
            engine.object_snapshot(tree).is_some(),
            "tree is not chopped"
        );

        let (mut no_tree, crew, tree, _linekit) = dig_double_linekit_consumption_fixture(false);
        no_tree.assign_object_removal(tree).test_value();
        let crew_index = no_tree.test_object_index(crew);
        no_tree.object_com_dig_double(crew_index).test_value();
        let crew_index = no_tree.test_object_index(crew);
        assert!(
            !no_tree.objects[crew_index]
                .state
                .local_vars
                .contains_key("own_activations"),
            "failed line construction does not fall through to own Activate"
        );
    }

    #[test]
    fn dig_double_rechecks_the_first_content_after_activate_replaces_it_with_linekit() {
        // The original first content removes itself after creating LNKT in
        // the Clonk. C++ re-reads Contents.GetObject() before the linekit
        // branch instead of testing the stale pre-Activate pointer
        // (C4ObjectCom.cpp:537-547).
        let (mut engine, crew, _tree, replaced) = dig_double_linekit_consumption_fixture(true);
        let crew_index = engine.test_object_index(crew);

        engine.object_com_dig_double(crew_index).test_value();

        assert!(
            engine
                .find_object_index(replaced)
                .is_none_or(|index| engine.objects[index].destroyed),
            "the activating content removes itself"
        );
        let crew_state = &test_object(&engine, crew);
        assert_eq!(
            crew_state.state.action.name, "Walk",
            "the freshly created LNKT runs ObjectComLineConstruction's Stand entry"
        );
        assert!(crew_state.commands.is_empty(), "no Chop command queued");
        assert!(
            !crew_state.state.local_vars.contains_key("own_activations"),
            "fresh LNKT consumes DigDouble before own Activate"
        );
        assert!(crew_state.state.contents.iter().any(|id| {
            engine.object_snapshot(*id).is_some_and(|object| {
                object.definition_id == "LNKT" && object.container == Some(crew)
            })
        }));
    }

    #[test]
    fn dig_double_keeps_the_entry_physical_backing_across_activate() {
        fn run_case(
            definition_can_chop: i32,
            temporary_can_chop: Option<i32>,
            activate_mode: i32,
            activate_can_chop: i32,
            actor_is_crew: bool,
            remove_actor: bool,
        ) -> Vec<String> {
            let mut engine = Engine::new();
            // This regression exercises the mutable Info->Physical backing.
            // Fair crew deliberately makes that backing read-only to scripts.
            engine.set_use_fair_crew(false);
            let mut clonk = test_definition("CLNK", "Clonk", "#strict\n");
            clonk.configure_actions(Some("Walk".to_owned()), clonk_actions());
            clonk.set_physical(PhysicalInfo {
                can_chop: definition_can_chop,
                ..Default::default()
            });
            engine.register_test_definition(clonk);

            let removal_target = if remove_actor { "clonk" } else { "this()" };
            let item_script = format!(
                r#"#strict 2
public func Activate(object clonk)
{{
  SetPhysical("CanChop", {activate_can_chop}, {activate_mode}, clonk);
  RemoveObject({removal_target});
  return(0);
}}
"#
            );
            engine.register_test_script_definition("ITEM", "Item", &item_script);
            let mut tree = test_definition("TREE", "Tree", "#strict\n");
            tree.set_chopable(true);
            tree.set_category(crate::CATEGORY_STATIC_BACK);
            tree.set_shape_rect(Some(crate::DefinitionRect::new(-10, -20, 20, 40)));
            engine.register_test_definition(tree);
            engine.register_test_player(PlayerConfig::new(1, "Test"));

            let command_crew = engine.spawn_test_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_position(Vector2::new(20, 20))
                    .with_action(ActionState::new("Walk")),
            );
            engine.select_crew(1, [command_crew]).test_value();
            engine.set_crew_cursor(1, Some(command_crew)).test_value();
            let actor = engine.spawn_test_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(actor_is_crew)
                    .with_position(Vector2::new(100, 100))
                    .with_action(ActionState::new("Walk"))
                    .with_loaded(true),
            );
            if actor_is_crew {
                let actor_index = engine.test_object_index(actor);
                engine.objects[actor_index].state.info_physical = Some(PhysicalInfo {
                    can_chop: definition_can_chop,
                    ..Default::default()
                });
            }
            let tree = engine.spawn_test_object(
                SpawnConfig::new("TREE")
                    .with_position(Vector2::new(100, 100))
                    .with_category(crate::CATEGORY_STATIC_BACK)
                    .with_loaded(true),
            );
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
            if let Some(can_chop) = temporary_can_chop {
                let actor_index = engine.test_object_index(actor);
                engine.objects[actor_index].state.temporary_physical = Some(PhysicalInfo {
                    can_chop,
                    ..Default::default()
                });
            }

            let actor_index = engine.test_object_index(actor);
            engine.object_com_dig_double(actor_index).test_value();
            assert_ne!(test_snapshot(&engine, tree).ocf & ocf::CHOP, 0);
            test_snapshot(&engine, command_crew)
                .command_stack
                .command_names()
        }

        assert_eq!(
            run_case(0, Some(0), 0, 1, false, false),
            vec!["Chop"],
            "PHYS_Current mutates the temporary backing captured at entry"
        );
        assert_eq!(
            run_case(1, None, 2, 0, false, false),
            vec!["Chop"],
            "enabling temporary physicals does not retarget the captured definition pointer"
        );
        assert_eq!(
            run_case(0, None, 0, 1, true, true),
            vec!["Chop"],
            "a captured Info physical remains valid after AssignRemoval retires its object"
        );
    }

    #[test]
    fn dig_double_does_not_call_activate_on_a_deleted_clonk() {
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict\npublic func Activate() { DoDamage(1); return(1); }\n",
        );
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
                public func Activate(object clonk)
                {
                  RemoveObject(clonk);
                  return(0);
                }
                "#,
        ));
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_action(ActionState::new("Walk")),
        );
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(crew));

        let crew_index = engine.test_object_index(crew);
        engine.object_com_dig_double(crew_index).test_value();

        let crew_index = engine.test_object_index(crew);
        assert!(engine.objects[crew_index].destroyed);
        assert_eq!(
            engine.objects[crew_index].state.damage, 0,
            "C4Object::Call returns nil without running scripts once Status is zero"
        );
    }

    #[test]
    fn line_message_to_deleted_target_only_clears_the_old_message() {
        let mut engine = Engine::new();
        engine.register_test_script_definition("TARG", "Target", "#strict\n");
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        engine.game_msg_object(target, "old".to_owned());
        assert_eq!(
            engine
                .messages
                .snapshot()
                .iter()
                .filter(|message| message.target == Some(target))
                .count(),
            1
        );
        let _ = test_object_mut(&mut engine, target).mark_destroyed();

        engine.game_msg_object(target, "new".to_owned());

        assert!(engine
            .messages
            .snapshot()
            .iter()
            .all(|message| message.target != Some(target)));
    }

    #[test]
    fn dig_double_with_linekit_starts_and_connects_power_line() {
        // When LNKT's script Activate does not consume DigDouble, C++ falls
        // through to ObjectComLineConstruction: a full-con structure under
        // the Clonk with C4D_Power_Output starts PWRL from that structure to
        // the carried kit (C4ObjectCom.cpp:542-547,487-528).
        let mut engine = Engine::new();
        register_builder_clonk(
            &mut engine,
            "CLNK",
            r#"#strict 2
local line_events;
public func NoteLineEvent(int event)
{
  line_events = line_events * 10 + event;
  return(1);
}
protected func Ejection(object item)
{
  NoteLineEvent(1);
  return(1);
}
"#,
        );

        let mut linekit = test_definition(
            "LNKT",
            "Linekit",
            r#"#strict 2
        local removal_observer;
        protected func Departure(object parent)
        {
          removal_observer = parent;
          parent->NoteLineEvent(2);
          return(1);
        }
        protected func Destruction()
        {
          removal_observer->NoteLineEvent(3);
          return(1);
        }
        "#,
        );
        linekit.set_shape_rect(Some(crate::DefinitionRect::new(-3, -12, 6, 28)));
        engine.register_test_definition(linekit);

        let mut generator = test_definition("POWR", "Generator", "#strict\n");
        generator.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        generator.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine.register_test_definition(generator);
        let mut consumer = test_definition("CONS", "Consumer", "#strict\n");
        consumer.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        consumer.set_line_connect(crate::LINE_CONNECT_POWER_INPUT);
        engine.register_test_definition(consumer);

        let mut line = test_definition(
            "PWRL",
            "Power line",
            r#"#strict 2
        static construction_vertex_count;
        static construction_vertex_x;
        static construction_vertex_y;
        static construction_width;
        static construction_height;
        public func Construction()
        {
          construction_vertex_count = GetVertexNum();
          construction_vertex_x = GetVertex(0, VTX_X);
          construction_vertex_y = GetVertex(0, VTX_Y);
          construction_width = GetObjWidth();
          construction_height = GetObjHeight();
          while (GetVertexNum()) RemoveVertex(0);
          return(1);
        }
        public func Initialize() { SetAction("Connect"); return(1); }
        "#,
        );
        line.set_line(1);
        line.set_shape_rect(Some(crate::DefinitionRect::new(-4, -6, 8, 12)));
        line.set_shape_vertices(vec![
            crate::ObjectVertex {
                x: 11,
                y: 12,
                cnat: crate::CNAT_LEFT,
                friction: 3,
            },
            crate::ObjectVertex {
                x: 21,
                y: 22,
                cnat: crate::CNAT_RIGHT,
                friction: 4,
            },
            crate::ObjectVertex {
                x: 777,
                y: 888,
                cnat: crate::CNAT_BOTTOM,
                friction: 9,
            },
        ]);
        configure_connect_action(&mut line);
        engine.register_test_definition(line);

        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let generator = engine.spawn_test_object(
            // NewObject's initial DoCon keeps the supplied bottom at
            // y=120, yielding a full-con centre at y=100.
            SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)),
        );
        let consumer = engine
            .spawn_test_object(SpawnConfig::new("CONS").with_position(Vector2::new(200, 120)));
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Walk")),
        );
        engine.select_crew(1, vec![crew]).test_value();
        engine.set_crew_cursor(1, Some(crew)).test_value();
        let kit =
            engine.spawn_test_object(SpawnConfig::new("LNKT").with_owner(1).with_container(crew));
        let generator_index = engine.test_object_index(generator);
        assert_ne!(
            engine.object_ocf_at_index(generator_index) & ocf::LINE_CONSTRUCT,
            0,
            "full-con power output advertises OCF_LineConstruct"
        );
        assert!(
            engine
                .at_object(Vector2::new(100, 100), ocf::LINE_CONSTRUCT, Some(crew))
                .is_some(),
            "the generator is under the Clonk's line-construction point"
        );

        engine.player_in_com(1, COM_DIG, 0).test_value();
        engine.player_in_com(1, COM_DIG, 0).test_value();

        let power_line = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "PWRL")
            .test_value();
        assert_eq!(power_line.action.name, "Connect");
        assert_eq!(power_line.action.target, Some(generator));
        assert_eq!(power_line.action.target2, Some(kit));
        assert_eq!(
            power_line
                .vertices
                .iter()
                .map(|vertex| (vertex.x, vertex.y, vertex.cnat, vertex.friction))
                .collect::<Vec<_>>(),
            vec![
                (100, 110, crate::CNAT_LEFT, 3),
                (100, 107, crate::CNAT_RIGHT, 4),
            ],
            "CreateLine installs exactly two endpoint positions while retaining dormant slot metadata"
        );
        let globals = &engine.snapshot().script_globals.named;
        assert_eq!(
            globals.get("construction_vertex_count"),
            Some(&Value::Int(3))
        );
        assert_eq!(globals.get("construction_vertex_x"), Some(&Value::Int(11)));
        assert_eq!(globals.get("construction_vertex_y"), Some(&Value::Int(12)));
        assert_eq!(globals.get("construction_width"), Some(&Value::Int(8)));
        assert_eq!(globals.get("construction_height"), Some(&Value::Int(12)));
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(generator)
                && message.lines == ["New".to_owned(), "Power line.".to_owned()]
        }));

        // At the other endpoint, the same real DigDouble accepts PWRL on a
        // C4D_Power_Input structure, swaps the kit endpoint, exits the kit,
        // and removes it (C4ObjectCom.cpp:429-484).
        engine
            .apply_object_update(
                kit,
                crate::ObjectUpdate::new().with_status(crate::ObjectStatus::Inactive),
            )
            .test_value();
        assert_eq!(
            test_snapshot(&engine, kit).status,
            crate::ObjectStatus::Inactive,
            "the completion path must find an inactive carried linekit"
        );
        engine
            .player_in_com(1, COM_DIG + COM_RELEASE_OFFSET, 0)
            .test_value();
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].set_position(Vector2::new(200, 100));
        engine.update_sector_for_index(crew_index);
        engine.player_in_com(1, COM_DIG, 0).test_value();
        engine
            .player_in_com(1, COM_DIG + COM_RELEASE_OFFSET, 0)
            .test_value();
        engine.player_in_com(1, COM_DIG, 0).test_value();

        let connected = test_snapshot(&engine, power_line.id);
        assert_eq!(connected.action.target, Some(generator));
        assert_eq!(connected.action.target2, Some(consumer));
        assert!(
            engine
                .find_object_index(kit)
                .is_none_or(|index| engine.objects[index].destroyed),
            "the connected LNKT is removed"
        );
        assert!(!test_snapshot(&engine, crew).contents.contains(&kit));
        let crew_index = engine.test_object_index(crew);
        assert_eq!(
            object_local(&engine, crew, "line_events"),
            Some(&Value::Int(123)),
            "Exit calls Ejection then Departure before AssignRemoval calls Destruction"
        );
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(consumer)
                && message.lines == ["Power line conntected".to_owned(), "to Consumer".to_owned()]
        }));
    }

    #[test]
    fn line_construction_stands_before_can_construct_failure() {
        // ObjectComLineConstruction always calls ObjectActionStand before
        // checking CanConstruct (C4ObjectCom.cpp:384-390).
        let mut engine = clonk_engine("#strict\n");
        engine.register_test_script_definition("LNKT", "Linekit", "#strict\n");
        let mut structure = test_definition("POWR", "Generator", "#strict\n");
        structure.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        structure.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine.register_test_definition(structure);
        let mut line = test_definition("PWRL", "Power line", "#strict\n");
        line.set_line(1);
        configure_connect_action(&mut line);
        engine.register_test_definition(line);

        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Jump"))
                .with_command_direction(CommandDirection::Right)
                .with_velocity(Vector2::new(7, -3)),
        );
        let kit = engine.spawn_test_object(SpawnConfig::new("LNKT").with_container(crew));
        let structure = engine
            .spawn_test_object(SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)));
        let crew_index = engine.test_object_index(crew);
        assert!(engine
            .at_object(Vector2::new(100, 100), ocf::LINE_CONSTRUCT, Some(crew),)
            .is_some_and(|(_, id, object_ocf)| {
                id == structure && object_ocf & ocf::LINE_CONSTRUCT != 0
            }));

        assert!(!engine
            .object_com_line_construction(crew_index)
            .expect("line construction returns normally"));

        let crew = test_snapshot(&engine, crew);
        assert_eq!(crew.action.name, "Walk");
        assert_eq!(crew.command_direction, CommandDirection::Stop);
        assert_eq!(crew.velocity, Vector2::ZERO);
        assert!(crew.contents.contains(&kit));
        assert!(engine
            .objects
            .iter()
            .all(|object| object.definition_id != "PWRL"));
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(crew.id)
                && message.lines == ["CLNK cannot create lines.".to_owned()]
        }));
    }

    #[test]
    fn line_short_circuit_calls_destruction() {
        // Reconnecting a carried line to its existing structure removes the
        // line through AssignRemoval, including Destruction
        // (C4ObjectCom.cpp:445-453).
        let mut engine = Engine::new();
        register_builder_clonk(
            &mut engine,
            "CLNK",
            r#"#strict 2
local line_destructions;
local destroyed_line;
local destruction_saw_line;
public func NoteLineDestruction(object line)
{
  line_destructions++;
  if (line) destruction_saw_line = 1;
  destroyed_line = line;
  return(1);
}
"#,
        );
        engine.register_test_script_definition("LNKT", "Linekit", "#strict\n");

        let mut structure = test_definition("POWR", "Generator", "#strict\n");
        structure.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        structure.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine.register_test_definition(structure);

        let mut line = test_definition(
            "PWRL",
            "Power line",
            r#"#strict 2
        local observer;
        local stop_order;
        local lower_saw_upper;
        local abort_saw_late;
        local late_stops;
        public func Arm(object target)
        {
          observer = target;
          AddEffect("Lower", this(), 100, 0, this());
          AddEffect("Upper", this(), 200, 0, this());
          return(1);
        }
        protected func FxUpperStop(object target, int number, int reason)
        {
          stop_order = stop_order * 10 + 2;
          return(-1);
        }
        protected func FxLowerStop(object target, int number, int reason)
        {
          stop_order = stop_order * 10 + 1;
          lower_saw_upper = !!GetEffect("Upper", target);
          AddEffect("Late", target, 1, 0, target);
          return(0);
        }
        protected func FxLateStop()
        {
          late_stops++;
          return(0);
        }
        protected func RemovedAbort()
        {
          stop_order = stop_order * 10 + 3;
          abort_saw_late = !!GetEffect("Late", this());
          return(1);
        }
        protected func Destruction()
        {
          observer->NoteLineDestruction(this());
          return(1);
        }
        "#,
        );
        line.set_c4_callback_convention(true);
        line.set_line(1);
        line.set_shape_vertices(vec![
            crate::ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
            crate::ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
        ]);
        line.configure_actions(
            Some("Connect".to_owned()),
            HashMap::from([(
                "Connect".to_owned(),
                ActionSpec::default()
                    .with_procedure("connect")
                    .with_abort_call("RemovedAbort"),
            )]),
        );
        engine.register_test_definition(line);

        let structure = engine
            .spawn_test_object(SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)));
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Walk")),
        );
        let kit = engine.spawn_test_object(SpawnConfig::new("LNKT").with_container(crew));
        let mut connect_action = ActionState::new("Connect");
        connect_action.target = Some(structure);
        connect_action.target2 = Some(kit);
        let line = engine.spawn_test_object(
            SpawnConfig::new("PWRL")
                .with_owner(1)
                .with_action(connect_action),
        );
        let line_index = engine.test_object_index(line);
        engine.call_test_object_function(
            line_index,
            "Arm",
            vec![compat::object_reference_value(crew)],
        );
        assert_eq!(
            engine.objects[line_index]
                .state
                .effects
                .iter()
                .map(|effect| (effect.name.as_str(), effect.command_target))
                .collect::<Vec<_>>(),
            vec![
                ("Lower", Some(line.as_u64() as i32)),
                ("Upper", Some(line.as_u64() as i32))
            ]
        );

        let crew_index = engine.test_object_index(crew);
        assert!(engine
            .object_com_line_construction(crew_index)
            .expect("short circuit completes"));

        assert!(engine
            .find_object_index(line)
            .is_none_or(|index| engine.objects[index].destroyed));
        assert_eq!(
            object_local(&engine, crew, "line_destructions"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            object_local(&engine, crew, "destroyed_line"),
            Some(&Value::Nil),
            "AssignRemoval clears C4Value references to the removed line synchronously"
        );
        assert_eq!(
            object_local(&engine, crew, "destruction_saw_line"),
            Some(&Value::Int(1)),
            "Destruction must observe its live argument before AssignRemoval clears it"
        );
        let line_locals = &test_object(&engine, line).state.local_vars;
        assert_eq!(line_locals.get("stop_order"), Some(&Value::Int(213)));
        assert_eq!(
            line_locals.get("lower_saw_upper"),
            Some(&Value::Bool(true)),
            "tail-first ClearAll keeps a denied later effect visible to the earlier Stop"
        );
        assert_eq!(
            line_locals.get("abort_saw_late"),
            Some(&Value::Bool(false)),
            "effects added by Stop are deleted before SetAction(Idle) runs AbortCall"
        );
        assert_eq!(
            line_locals.get("late_stops"),
            Some(&Value::Nil),
            "the post-ClearAll list deletion emits no second Stop callback"
        );
        assert!(test_object(&engine, crew).state.contents.contains(&kit));
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(structure)
                && message.lines == ["Power line disconnected.".to_owned()]
        }));
    }

    #[test]
    fn dig_double_without_kit_picks_up_connected_line() {
        // With empty contents, DigDouble over a line-connect structure calls
        // ObjectComLineConstruction's pickup half: create a kit owned by the
        // line owner, collect it, and retarget the structure endpoint
        // (C4ObjectCom.cpp:559-567,392-427).
        let (mut engine, crew, line, _structure, endpoint) =
            line_pickup_gate_fixture(0, false, false);

        let crew_index = engine.test_object_index(crew);
        engine.object_com_dig_double(crew_index).test_value();

        let crew_snapshot = test_snapshot(&engine, crew);
        let kits = crew_snapshot
            .contents
            .iter()
            .filter_map(|id| engine.object_snapshot(*id))
            .filter(|object| object.definition_id == "LNKT")
            .collect::<Vec<_>>();
        assert_eq!(kits.len(), 1, "pickup creates and collects one linekit");
        assert_eq!(kits[0].owner, 2, "the line owner owns the new kit");
        assert_eq!(kits[0].container, Some(crew));
        let line = test_snapshot(&engine, line);
        assert_eq!(line.action.target, Some(kits[0].id));
        assert_eq!(line.action.target2, Some(endpoint));
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(_structure)
                && message.lines
                    == [
                        "Power line disconnected".to_owned(),
                        "from Generator.".to_owned(),
                    ]
        }));
    }

    #[test]
    fn line_pickup_enters_an_inactive_kit_in_cpp_callback_order() {
        let clonk_script = r#"#strict 2
protected func RejectCollect(item_id, object item)
{
  item->NoteEnter(2);
  return(0);
}
protected func Collection2(object item)
{
  item->NoteEnter(3);
  return(1);
}
"#;
        let linekit_script = r#"#strict 2
local enter_order;
public func Construction()
{
  CreateMenu(LNKT, this(), this(), 0, "Kit");
  SetObjectStatus(2);
  return(1);
}
public func NoteEnter(int digit)
{
  enter_order = enter_order * 10 + digit;
  return(1);
}
protected func RejectEntrance()
{
  NoteEnter(1);
  return(0);
}
protected func Entrance()
{
  NoteEnter(4);
  return(1);
}
"#;
        let (mut engine, crew, line, structure, endpoint) =
            line_pickup_gate_fixture_with_linekit(0, false, false, clonk_script, linekit_script);
        let linekit_definition = engine.definitions.get_mut("LNKT").test_value();
        linekit_definition.set_shape_rect(Some(crate::DefinitionRect::new(-2, -2, 4, 4)));
        linekit_definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 4, 4, 0, 0)));
        let densities = vec![0, 100, 100];
        let names = vec![None, Some("Earth".to_owned()), Some("Vehicle".to_owned())];
        let grid = crate::landscape::PixelGrid::new(
            256,
            256,
            vec![0; 256 * 256],
            densities,
            names,
            vec![None; 3],
        );
        let mut landscape = Landscape::new(256, vec![256; 256]).test_value();
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let crew_index = engine.test_object_index(crew);
        assert!(engine
            .object_com_line_construction(crew_index)
            .expect("inactive pickup returns normally"));

        let kit = test_snapshot(&engine, crew)
            .contents
            .into_iter()
            .find(|object_id| {
                engine
                    .object_snapshot(*object_id)
                    .is_some_and(|object| object.definition_id == "LNKT")
            })
            .test_value();
        let kit_index = engine.test_object_index(kit);
        assert_eq!(
            engine.objects[kit_index].state.status,
            crate::ObjectStatus::Inactive
        );
        assert_eq!(engine.objects[kit_index].state.container, Some(crew));
        assert_eq!(
            object_local(&engine, kit, "enter_order"),
            Some(&Value::Int(1234)),
            "RejectEntrance, RejectCollect, Collection2, and Entrance keep C++ order"
        );
        assert!(
            engine.objects[kit_index].state.menu.is_none(),
            "Enter forcibly closes the entering object's menu before linking"
        );
        assert!(
            engine.objects[kit_index].solid_mask_bake.is_none(),
            "Enter removes the old-position solid mask before CopyMotion"
        );
        let line = test_snapshot(&engine, line);
        assert_eq!(line.action.target, Some(kit));
        assert_eq!(line.action.target2, Some(endpoint));
        assert_ne!(line.action.target, Some(structure));
    }

    #[test]
    fn line_pickup_respects_collection_limit_before_creating_a_kit() {
        // The nonzero CollectionLimit gate precedes AtObject, line lookup,
        // and linekit creation (C4ObjectCom.cpp:394-395).
        let (mut engine, crew, line, structure, endpoint) =
            line_pickup_gate_fixture(1, true, false);
        let crew_index = engine.test_object_index(crew);

        assert!(!engine
            .object_com_line_construction(crew_index)
            .expect("collection-limit rejection returns normally"));

        let line = test_snapshot(&engine, line);
        assert_eq!(line.action.target, Some(structure));
        assert_eq!(line.action.target2, Some(endpoint));
        assert!(engine
            .objects
            .iter()
            .all(|object| object.definition_id != "LNKT"));
    }

    #[test]
    fn line_pickup_rejects_a_line_already_ending_in_a_kit() {
        // A kit at either endpoint prevents a second pickup kit
        // (C4ObjectCom.cpp:404-410).
        let (mut engine, crew, line, structure, endpoint_kit) =
            line_pickup_gate_fixture(0, false, true);
        let crew_index = engine.test_object_index(crew);

        assert!(!engine
            .object_com_line_construction(crew_index)
            .expect("double-kit rejection returns normally"));

        let line = test_snapshot(&engine, line);
        assert_eq!(line.action.target, Some(structure));
        assert_eq!(line.action.target2, Some(endpoint_kit));
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "LNKT" && !object.destroyed)
                .count(),
            1,
            "no second linekit is created"
        );
        assert!(engine.messages.snapshot().iter().any(|message| {
            message.target == Some(crew)
                && message.lines == ["Power line is not fixed at the other end.".to_owned()]
        }));
    }

    #[test]
    fn rejected_linekit_pickup_destroys_the_uncollected_kit() {
        // Enter receives a RejectCollect result pointer. A collector veto
        // aborts pickup and the freshly-created kit is AssignRemoval'd before
        // the line can be retargeted (C4ObjectCom.cpp:412-418).
        let mut engine = Engine::new();
        register_builder_clonk(
            &mut engine,
            "CLNK",
            r#"#strict 2
local rejected_kit_destructions;
protected func RejectCollect(item_id, object item)
{
  item->Arm(this());
  return(1);
}
public func NoteRejectedKitDestruction()
{
  rejected_kit_destructions++;
  return(1);
}
"#,
        );
        engine.register_test_definition(test_definition(
            "LNKT",
            "Linekit",
            r#"#strict 2
                local observer;
                public func Arm(object target)
                {
                  observer = target;
                  return(1);
                }
                protected func Destruction()
                {
                  observer->NoteRejectedKitDestruction();
                  return(1);
                }
                "#,
        ));

        let mut structure = test_definition("POWR", "Generator", "#strict\n");
        structure.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        structure.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine.register_test_definition(structure);
        let mut endpoint = test_definition("CONS", "Consumer", "#strict\n");
        endpoint.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        engine.register_test_definition(endpoint);
        let mut line = test_definition("PWRL", "Power line", "#strict\n");
        line.set_line(1);
        configure_connect_action(&mut line);
        engine.register_test_definition(line);

        let structure = engine
            .spawn_test_object(SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)));
        let endpoint = engine
            .spawn_test_object(SpawnConfig::new("CONS").with_position(Vector2::new(200, 120)));
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_position(Vector2::new(100, 100))
                .with_action(ActionState::new("Walk")),
        );
        let mut connect_action = ActionState::new("Connect");
        connect_action.target = Some(structure);
        connect_action.target2 = Some(endpoint);
        let line = engine.spawn_test_object(
            SpawnConfig::new("PWRL")
                .with_owner(2)
                .with_action(connect_action),
        );

        let crew_index = engine.test_object_index(crew);
        assert!(!engine
            .object_com_line_construction(crew_index)
            .expect("pickup rejection returns normally"));

        let line = test_snapshot(&engine, line);
        assert_eq!(line.action.target, Some(structure));
        assert_eq!(line.action.target2, Some(endpoint));
        assert!(test_object(&engine, crew).state.contents.is_empty());
        assert_eq!(
            object_local(&engine, crew, "rejected_kit_destructions"),
            Some(&Value::Int(1))
        );
        assert!(engine
            .objects
            .iter()
            .all(|object| { object.definition_id != "LNKT" || object.destroyed }));
    }

    #[test]
    fn throw_com_queues_throw_command_for_the_cursor() {
        // DFA_WALK COM_Throw → PlayerObjectCommand(C4CMD_Throw)
        // (C4Object.cpp:3423, C4ObjectCom.cpp:1013-1040).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");

        engine.player_in_com(1, COM_THROW, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.command_stack.command_names(), vec!["Throw"]);
    }

    #[test]
    fn down_double_after_throw_converts_to_drop() {
        // LastComDownDouble makes the next throw a drop
        // (PlayerObjectCommand, C4ObjectCom.cpp:1020-1036).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");

        engine.player_in_com(1, COM_DOWN, 0).test_value();
        engine.player_in_com(1, COM_DOWN, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_stack.command_names(),
            vec!["Drop"],
            "down-down-throw is the classic drop (C4ObjectCom.cpp:1024-1036)"
        );
    }

    #[test]
    fn contained_com_down_issues_exit_command() {
        // ContainedControl hardcoded COM_Down → PlayerObjectCommand(
        // C4CMD_Exit) (C4Object.cpp:3256-3258).
        let hut = r#"
#strict
"#;
        let mut engine = clonk_engine("#strict\n");
        let hut_def = test_definition("HUT1", "Hut", hut);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_DOWN, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.command_stack.command_names(), vec!["Exit"]);
    }

    #[test]
    fn contained_com_left_issues_take_command() {
        // At 4.9.1.3+, a falsy ContainedLeft still reaches the hardcoded
        // Take/Take2 tail (C4Object.cpp:3246-3251,3293-3302).
        let mut engine = clonk_engine("#strict\n");
        let mut hut_def = test_definition(
            "HUT1",
            "Hut",
            "#strict\nprotected func ContainedLeft(pByClonk) { return(0); }\n",
        );
        hut_def.set_version([4, 9, 1, 3, 0]);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_LEFT, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.command_stack.command_names(), vec!["Take"]);
    }

    #[test]
    fn contained_control_zero_payload_id_result_does_not_consume_control() {
        // C4Value::operator bool tests the complete raw payload. C4ID_None
        // is therefore false even though it remains a typed C4ID value
        // (C4Value.h:76,183-185).
        let mut engine = clonk_engine("#strict\n");
        let mut hut_definition = test_definition(
            "HUT1",
            "Hut",
            r#"#strict 2
            protected func ContainedLeft(object driver)
            {
                return C4Id("NONE");
            }
        "#,
        );
        hut_definition.set_version([4, 9, 1, 3, 0]);
        engine.register_test_definition(hut_definition);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_LEFT, 0).test_value();

        assert_eq!(
            test_snapshot(&engine, crew).command_stack.command_names(),
            vec!["Take"],
            "a zero-payload C4ID result is false and reaches the hardcoded Take"
        );
    }

    #[test]
    fn contained_control_direct_function_runs_on_status_zero_container() {
        let mut engine = clonk_engine("#strict\n");
        let mut hut_def = test_definition(
            "HUT1",
            "Hut",
            r#"
            #strict 2
            local direct_calls;
            protected func ContainedLeft(object driver)
            {
                direct_calls++;
                return true;
            }
        "#,
        );
        hut_def.set_version([4, 9, 1, 3, 0]);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");
        let hut_index = engine.test_object_index(hut);
        let _ = engine.objects[hut_index].mark_destroyed();

        engine.player_in_com(1, COM_LEFT, 0).test_value();

        let hut_index = engine.test_object_index(hut);
        assert_eq!(
            object_local(&engine, hut, "direct_calls"),
            Some(&Value::Int(1)),
            "sf->Exec bypasses C4Object::Call's Status gate"
        );
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "the truthy direct handler consumes the control before Take"
        );
    }

    #[test]
    fn old_contained_left_function_suppresses_take_even_when_falsy() {
        // Before 4.9.1.3 any ContainedLeft function suppresses the Take
        // fallback because the callback runs after hardcoded controls
        // (src/C4Object.cpp:3284-3302).
        let mut engine = clonk_engine("#strict\n");
        let mut hut_def = test_definition(
            "HUT1",
            "Hut",
            "#strict\nprotected func ContainedLeft(pByClonk) { DoDamage(1); return(0); }\n",
        );
        hut_def.set_version([4, 9, 1, 2, 0]);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_LEFT, 0).test_value();

        assert_eq!(test_snapshot(&engine, hut).damage, 1);
        assert!(
            test_snapshot(&engine, crew)
                .command_stack
                .command_names()
                .is_empty(),
            "the presence of an old late ContainedLeft suppresses Take"
        );
    }

    #[test]
    fn contained_control_update_uses_native_name_and_arguments_for_old_and_new_definitions() {
        // PSF_ContainedControlUpdate is the internal C++ identifier, but its
        // script callback is `~ContainedUpdate`. Both the early and late
        // version branches pass (driver, comdir, dig, throw)
        // (C4Script.h:74; C4Object.cpp:3253-3263,3296-3305).
        let container_script = r#"
#strict 2
local update_count, update_driver, update_dir, update_dig, update_throw, wrong_name;
protected func ContainedUpdate(object driver, int dir, bool digging, bool throwing)
{
    update_count++;
    update_driver = driver;
    update_dir = dir + 100;
    update_dig = digging;
    update_throw = throwing;
    return(1);
}
protected func ContainedControlUpdate()
{
    wrong_name = 1;
    return(1);
}
"#;

        for (label, version) in [
            ("modern early callback", [4, 9, 1, 3, 0]),
            ("legacy late callback", [4, 9, 1, 2, 0]),
        ] {
            let mut engine = clonk_engine("#strict\n");
            let mut container = test_definition("CONT", "Container", container_script);
            container.set_version(version);
            engine.register_test_definition(container);
            let (crew, container) = contain_player_crew(&mut engine, "CONT");
            let player = engine.player_mut(1).test_value();
            player.control.control_style = true;
            player.control.pressed_coms = (1_i32 << COM_DIG) | (1_i32 << COM_THROW);

            engine.player_in_com(1, COM_LEFT, 0).test_value();

            let container = test_snapshot(&engine, container);
            assert_eq!(
                container.local_vars.get("update_count"),
                Some(&Value::Int(1)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_driver"),
                Some(&Value::Object(crew.as_u64())),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_dir"),
                Some(&Value::Int(CommandDirection::Left.to_script_value() + 100)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_dig"),
                Some(&Value::Bool(true)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_throw"),
                Some(&Value::Bool(true)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("wrong_name"),
                Some(&Value::Nil),
                "{label}"
            );

            engine
                .player_in_com(1, COM_LEFT + COM_RELEASE_OFFSET, 0)
                .test_value();
            let container = test_snapshot(&engine, container.id);
            assert_eq!(
                container.local_vars.get("update_count"),
                Some(&Value::Int(2)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_dir"),
                Some(&Value::Int(CommandDirection::Stop.to_script_value() + 100)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_dig"),
                Some(&Value::Bool(true)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("update_throw"),
                Some(&Value::Bool(true)),
                "{label}"
            );
            assert_eq!(
                container.local_vars.get("wrong_name"),
                Some(&Value::Nil),
                "{label}"
            );
        }
    }

    #[test]
    fn contained_throw_executes_the_new_command_immediately() {
        // C4Object::ContainedControl evaluates
        // `PlayerObjectCommand(..., C4CMD_Throw) && ExecuteCommand()` in
        // one control call (C4Object.cpp:3280-3282). The completed Throw
        // command is therefore gone before control returns.
        for command in [COM_THROW, COM_THROW_D] {
            let mut engine = clonk_engine("#strict\n");
            engine.register_test_script_definition("HUT1", "Hut", "#strict\n");
            let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

            engine.player_in_com(1, command, 0).test_value();

            let snapshot = test_snapshot(&engine, crew);
            assert!(
                snapshot.command_stack.is_empty(),
                "ContainedControl executes and clears Throw synchronously"
            );
        }
    }

    #[test]
    fn kayak_contained_throw_opens_and_executes_the_explicit_activate_menu() {
        // Reduced shipped KAJO::ContainedThrow: a full kayak queues an
        // Activate command on its contained Clonk with the kayak in Target2.
        // C4MN_Activate groups visible cargo, excludes NoGet definitions,
        // and keeps the permanent menu refilled after selected cargo exits
        // (FarWorlds.../Kajak.c4d/Occupied.c4d/Script.c:123-133;
        // C4Object.cpp:1884-1918; C4ObjectMenu.cpp:170-205,448-459).
        let kayak = r#"
#strict 2
protected func ContainedThrow(object clonk)
{
    return AddCommand(clonk, "Activate", 0, 0, 0, this());
}
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut kayak_definition = test_definition("KAJO", "Occupied kayak", kayak);
        kayak_definition.set_entrance_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        engine.register_test_definition(kayak_definition);
        let mut cargo = test_definition(
            "CRGO",
            "Cargo",
            "#strict 2\nfunc CalcValue() { return 41; }\n",
        );
        cargo.set_category(crate::CATEGORY_OBJECT);
        cargo.set_description(Some("Kayak cargo.".to_string()));
        engine.register_test_definition(cargo);
        let mut hidden = test_definition("HIDN", "Hidden cargo", "#strict 2\n");
        hidden.set_category(crate::CATEGORY_OBJECT);
        hidden.set_no_get(true);
        engine.register_test_definition(hidden);
        engine.register_test_player(PlayerConfig::new(1, "Paddler"));
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.category = crate::CATEGORY_LIVING;
        let kayak = engine.spawn_test_object(SpawnConfig::new("KAJO").with_owner(1));
        test_object_mut(&mut engine, kayak).state.entrance_status = true;
        let cargo = [
            engine.spawn_test_object(SpawnConfig::new("CRGO").with_container(kayak)),
            engine.spawn_test_object(SpawnConfig::new("CRGO").with_container(kayak)),
        ];
        engine.spawn_test_object(SpawnConfig::new("HIDN").with_container(kayak));
        contain_object(&mut engine, crew, kayak);

        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(
            test_snapshot(&engine, crew).command_stack.command_names(),
            ["Activate"]
        );

        engine.execute_object_command_now(crew).test_value();

        assert!(test_snapshot(&engine, crew).command_stack.is_empty());
        assert!(engine.pending_menu_requests.is_empty());
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(6));
        assert_eq!(menu.refill_object, Some(kayak));
        assert_eq!(menu.refill_object_contents_count, 0);
        assert_eq!(menu.caption, "Occupied kayak is empty.");
        assert_eq!(menu.items.len(), 1, "NoGet cargo stays hidden");
        let item = &menu.items[0];
        assert_eq!(item.caption, "Activate Cargo");
        assert_eq!(item.info_caption, "Kayak cargo.");
        assert_eq!(item.count, 2);
        assert_eq!(
            item.picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("CRGO"),
            "C4ObjectMenu::RefillInternal captures the symbol before later ticks",
        );
        let selected_cargo = item.picture_object.test_value();
        assert!(cargo.contains(&selected_cargo));
        let remaining_cargo = cargo
            .into_iter()
            .find(|candidate| *candidate != selected_cargo)
            .test_value();
        assert_eq!(item.value, Some(41));
        assert_eq!(
            item.command,
            format!(
                "SetCommand(this,\"Activate\",Object({}))&&ExecuteCommand()",
                selected_cargo.as_u64()
            )
        );
        assert_eq!(
            item.command2,
            format!(
                "SetCommand(this,\"Activate\", ,2,0,Object({}),CRGO)&&ExecuteCommand()",
                kayak.as_u64()
            )
        );

        engine.execute_player_controls().test_value();
        assert_eq!(
            test_menu(&engine, crew).refill_object_contents_count,
            4,
            "the count includes the contained Clonk and NoGet cargo"
        );

        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(
            test_snapshot(&engine, selected_cargo)
                .command_stack
                .command_names(),
            ["Exit"]
        );
        engine.tick_without_snapshot().test_value();
        let selected_after_evaluation = test_snapshot(&engine, selected_cargo);
        assert_eq!(
            selected_after_evaluation.container,
            Some(kayak),
            "Exit's InitEvaluation consumes the first cargo execution"
        );
        assert_eq!(
            selected_after_evaluation.command_stack.command_names(),
            ["Exit"]
        );
        engine.tick_without_snapshot().test_value();
        assert_eq!(test_snapshot(&engine, selected_cargo).container, None);
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].count, 1);
        assert_eq!(menu.items[0].picture_object, Some(remaining_cargo));
        assert_eq!(
            menu.items[0]
                .picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("CRGO"),
            "the surviving row keeps its refill-time picture inputs",
        );
    }

    #[test]
    fn internal_menus_test_only_the_first_full_con_picture_candidate() {
        // C4ObjectMenu's "easy way" calls Contents.Find once and tests only
        // that first FULL_CON object's picture. It must not search through a
        // non-concatenating full object for a later concatenating one
        // (C4ObjectMenu.cpp:183-189,252-259,292-313).
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        engine.register_test_player(PlayerConfig::new(1, "Test"));

        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        // Same-ID contents insert at their cluster head. Spawn in reverse so
        // the final list is incomplete-red, blue, blue, full-red. Besides
        // the first-full candidate rule, this pins C++'s literal pCheck
        // reset/increment behavior on alternating picture groups.
        let later_full_red =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let second_full_blue =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let first_full_blue =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        for blue in [first_full_blue, second_full_blue] {
            set_second_picture_row(&mut engine, blue);
        }
        let incomplete_red = engine.spawn_test_object(
            SpawnConfig::new("ITEM")
                .with_construction(crate::FULL_CON / 2)
                .with_container(container),
        );
        assert_eq!(
            test_snapshot(&engine, container).contents,
            [
                incomplete_red,
                first_full_blue,
                second_full_blue,
                later_full_red,
            ]
        );
        assert!(!engine.can_concat_picture_with(
            &test_snapshot(&engine, incomplete_red),
            &test_snapshot(&engine, first_full_blue),
        ));
        assert!(engine.can_concat_picture_with(
            &test_snapshot(&engine, incomplete_red),
            &test_snapshot(&engine, later_full_red),
        ));

        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        let contents_menu = open_contents_test_menu(&mut engine, crew, container, 18);
        assert_eq!(
            menu_picture_objects(&contents_menu),
            vec![
                Some(incomplete_red),
                Some(first_full_blue),
                Some(later_full_red),
            ]
        );
        assert!(
            menu_pictures_are_owned(&contents_menu),
            "C4ObjectMenu.cpp:311-313 owns every Contents row picture at refill",
        );

        let activate_menu = open_activate_test_menu(&mut engine, crew, container);
        assert_eq!(
            menu_picture_objects(&activate_menu),
            vec![
                Some(incomplete_red),
                Some(first_full_blue),
                Some(later_full_red),
            ]
        );
        assert!(
            menu_pictures_are_owned(&activate_menu),
            "C4ObjectMenu.cpp:194-199 owns every Activate row picture at refill",
        );

        engine.objects[container_index].state.base = 1;
        engine
            .open_base_sell_menu(crew_index, container_index)
            .test_value();
        let sell_menu = test_menu(&engine, crew);
        assert_eq!(
            menu_picture_objects(&sell_menu),
            vec![
                Some(incomplete_red),
                Some(first_full_blue),
                Some(later_full_red),
            ]
        );
        assert!(
            menu_pictures_are_owned(&sell_menu),
            "C4ObjectMenu.cpp:258-263 owns every Sell row picture at refill",
        );

        // C4Object::AssignRemoval clears menu object references before the
        // later tick snapshot, but the facet copied by Add remains owned by
        // the row (C4Object.cpp:302-304; C4Menu.cpp:388-398).
        let removed_source = sell_menu.items[2].picture_object.test_value();
        engine
            .clear_object_references_for_removal(removed_source)
            .test_value();
        let menu_after_removal = engine
            .debug_object_menu(crew.as_u64())
            .test_value()
            .test_value();
        assert_eq!(menu_after_removal.items[2].picture_object, None);
        assert_eq!(
            menu_after_removal.items[2]
                .picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("ITEM"),
            "the refill-time picture survives removal cleanup",
        );
    }

    #[test]
    fn status_zero_menu_links_stay_iterator_visible_but_not_find_or_count_visible() {
        // AssignRemoval sets Status=0 before it runs contained-object
        // callbacks and only unlinks itself from its own container later.
        // C4ObjectListIterator therefore still yields and groups that raw
        // link, while Contents.Find/ObjectCount deliberately skip it.
        let mut engine = Engine::new();
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        for id in ["RAW1", "RAW2"] {
            let mut item = test_definition(id, id, "#strict 2\n");
            item.set_category(crate::CATEGORY_OBJECT);
            engine.register_test_definition(item);
        }

        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let raw1_live =
            engine.spawn_test_object(SpawnConfig::new("RAW1").with_container(container));
        let raw1_status_zero =
            engine.spawn_test_object(SpawnConfig::new("RAW1").with_container(container));
        let raw2_live_full =
            engine.spawn_test_object(SpawnConfig::new("RAW2").with_container(container));
        let raw2_status_zero_full =
            engine.spawn_test_object(SpawnConfig::new("RAW2").with_container(container));
        let raw2_live_incomplete = engine.spawn_test_object(
            SpawnConfig::new("RAW2")
                .with_construction(crate::FULL_CON / 2)
                .with_container(container),
        );

        for object in [raw1_status_zero, raw2_status_zero_full] {
            let index = engine.test_object_index(object);
            engine.objects[index].state.status = crate::ObjectStatus::Deleted;
            engine.objects[index].destroyed = true;
            assert_eq!(engine.objects[index].state.container, Some(container));
        }
        let contents = test_snapshot(&engine, container).contents;
        assert!(contents.contains(&raw1_status_zero));
        assert!(contents.contains(&raw2_status_zero_full));

        let source = EngineInternalObjectMenuSource(&mut engine);
        let groups =
            internal_object_menu_picture_groups(&source, &contents, crate::CATEGORY_OBJECT);
        assert!(
            groups
                .iter()
                .any(|group| { group.representative == raw1_status_zero && group.count == 2 }),
            "a full-con Status-zero group head remains the raw representative"
        );
        assert!(groups.iter().any(|group| {
            group.representative == raw2_live_full && group.count == 3
        }), "grouping counts the raw Status-zero link, but Contents.Find skips it when choosing the full-con representative");

        assert_eq!(
            internal_live_contents_definition_count(&source, &contents, "RAW1"),
            1,
            "Contents.ObjectCount skips the Status-zero RAW1 link"
        );
        assert_eq!(
            internal_live_contents_definition_count(&source, &contents, "RAW2"),
            2,
            "Contents.ObjectCount keeps only the two live RAW2 links"
        );
        assert_eq!(
            internal_live_contents_count(&source, &contents),
            3,
            "the unfiltered ObjectCount form also skips both Status-zero links"
        );

        // Keep all identities live in the assertion diagnostics and make the
        // intended same-ID grouping explicit.
        assert_ne!(raw1_live, raw1_status_zero);
        assert_ne!(raw2_live_incomplete, raw2_status_zero_full);
    }

    #[test]
    fn contents_menu_includes_inactive_links_and_runs_collection_gates() {
        let mut engine = Engine::new();
        let crew_script = r#"#strict 2
local reject_calls;
protected func RejectCollect(item_id, object item)
{
  reject_calls++;
  return 1;
}
"#;
        let mut crew_definition = test_definition("CLNK", "Clonk", crew_script);
        crew_definition.set_collection_limit(1);
        engine.register_test_definition(crew_definition);
        let mut container = structure_definition("CONT", "Container", "#strict 2\n");
        container.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(container);
        for id in ["ITEM", "FILL"] {
            let mut definition = test_definition(id, id, "#strict 2\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            definition.set_collectible(true);
            engine.register_test_definition(definition);
        }
        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        engine.spawn_test_object(SpawnConfig::new("FILL").with_container(crew));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let container_index = engine.test_object_index(container);
        engine.objects[container_index].state.entrance_status = true;
        engine.refresh_object_ocf(container_index);
        let item = engine.spawn_test_object(
            SpawnConfig::new("ITEM")
                .with_status(crate::ObjectStatus::Inactive)
                .with_container(container),
        );
        let crew_index = engine.test_object_index(crew);

        let menu = open_contents_test_menu(&mut engine, crew, container, 18);
        assert_eq!(menu.items.len(), 1, "Status=Inactive remains in Contents");
        assert_eq!(menu.items[0].picture_object, Some(item));
        assert!(menu.items[0].command.contains("\"Activate\""));
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(1))
        );

        engine
            .initialize_container_contents_menu(crew_index, container_index, 13)
            .test_value();
        let get_menu = test_menu(&engine, crew);
        assert_eq!(get_menu.items.len(), 1);
        assert!(get_menu.items[0].command.contains("\"Get\""));
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(1)),
            "C4MN_Get does not ask RejectCollect"
        );
    }

    #[test]
    fn contents_menu_preserves_stale_command2_and_cpp_selection_adjustment() {
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        for (id, name) in [("MULT", "Multiple"), ("SING", "Single")] {
            let mut definition = test_definition(id, name, "#strict 2\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            definition.set_collectible(true);
            engine.register_test_definition(definition);
        }
        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        engine.spawn_test_object(SpawnConfig::new("SING").with_container(container));
        engine.spawn_test_object(SpawnConfig::new("MULT").with_container(container));
        engine.spawn_test_object(SpawnConfig::new("MULT").with_container(container));
        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .initialize_container_contents_menu(crew_index, container_index, 13)
            .test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_item_counts(&menu), [("MULT", 2), ("SING", 1)]);
        assert!(!menu.items[0].command2.is_empty());
        assert_eq!(
            menu.items[1].command2, menu.items[0].command2,
            "C4ObjectMenu reuses the prior multi-count command2 for a later singleton row"
        );

        let mut none_selection = menu.clone();
        none_selection.selection = 1;
        none_selection.items[1].item_id = "NONE".to_string();
        assert_eq!(
            internal_object_menu_selected_definition(&none_selection),
            None,
            "C4ID_None bypasses checkIDSelection"
        );
        let mut rows = menu.items.clone();
        rows[1].selectable = false;
        assert_eq!(
            internal_refilled_object_menu_selection(&rows, Some(1), None),
            0,
            "AdjustSelection searches downward first"
        );
        rows[0].selectable = false;
        rows[1].selectable = true;
        assert_eq!(
            internal_refilled_object_menu_selection(&rows, Some(0), None),
            1,
            "AdjustSelection searches upward after exhausting lower slots"
        );
    }

    #[test]
    fn activate_refill_tracks_link_incarnation_and_both_cpp_iterators() {
        // First row A (red) exits and re-enters at the same final list slot
        // from CalcValue. C++'s pCurr and pCurrID stay on old successor B
        // (blue), so GetNext increments to C (red), whose picture is compared
        // against B and emitted. An ObjectId cursor would alias A's new link
        // and incorrectly emit B instead (C4ObjectList.cpp:249-253,849-903).
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let script = r#"#strict 2
local relink;
protected func CalcValue(object pInBase)
{
  if (relink)
  {
    relink = 0;
    Exit();
    Enter(pInBase);
  }
  return 9;
}
"#;
        let mut item = test_definition("ITEM", "Item", script);
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let c_red = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let b_blue = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        set_second_picture_row(&mut engine, b_blue);
        let a_red = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let a_index = engine.test_object_index(a_red);
        engine.objects[a_index]
            .state
            .local_vars
            .insert("relink".to_string(), Value::Int(1));
        let generation_before = engine.objects[a_index].state.contents_link_generation;
        assert_eq!(
            test_snapshot(&engine, container).contents,
            [a_red, b_blue, c_red]
        );

        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_activate_menu(crew_index, container_index)
            .test_value();

        assert_ne!(
            test_object(&engine, a_red).state.contents_link_generation,
            generation_before,
            "Exit+Enter allocates a distinct C4ObjectList link"
        );
        assert_eq!(
            test_snapshot(&engine, container).contents,
            [a_red, b_blue, c_red],
            "the final id vector can be identical despite new link identity"
        );
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_picture_objects(&menu), vec![Some(a_red), Some(c_red)]);
    }

    #[test]
    fn callback_exit_cycles_preserve_every_contents_link_incarnation() {
        // Every successful Enter allocates a fresh Contents link before its
        // synchronous Collection2 callback; each Exit then deletes that link
        // (C4Object.cpp:1542-1547,1598-1627;
        // C4ObjectList.cpp:129-132,240-259). Copy-out must retain both
        // allocations even though the final relationship is free -> free.
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition(
            "CONT",
            "Container",
            r#"#strict 2
local cycling;
protected func Collection2(object item)
{
  if (cycling) return 1;
  cycling = 1;
  item->Exit();
  item->Enter(this());
  item->Exit();
  return 1;
}
"#,
        );
        engine.register_test_definition(container);
        let item = test_definition(
            "ITEM",
            "Item",
            "#strict 2\npublic func Cycle(object target) { return Enter(target); }\n",
        );
        engine.register_test_definition(item);
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));
        let item_index = engine.test_object_index(item);
        let generation_before = engine.objects[item_index].state.contents_link_generation;

        assert_eq!(
            engine
                .call_object_function(
                    item_index,
                    "Cycle",
                    vec![crate::compat::object_reference_value(container)],
                )
                .test_value(),
            Value::Bool(true)
        );

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, None);
        assert!(test_snapshot(&engine, container).contents.is_empty());
        assert_eq!(
            item.state.contents_link_generation,
            generation_before + 2,
            "both transient C4ObjectLink allocations remain observable"
        );
    }

    #[test]
    fn transient_enter_retains_link_incarnation_after_pending_container_removal() {
        // Enter allocates the link before Exit deletes it; removing the now
        // empty container afterward cannot erase that completed incarnation
        // (C4Object.cpp:1542-1547,1598-1605,287-306;
        // C4ObjectList.cpp:129-132,240-259).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
public func Cycle()
{
  var target = CreateObject(CONT);
  Enter(target);
  Exit();
  target->RemoveObject();
  return 1;
}
"#,
        ));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));
        let item_index = engine.test_object_index(item);
        let generation_before = engine.objects[item_index].state.contents_link_generation;

        assert_eq!(
            engine
                .call_object_function(item_index, "Cycle", Vec::new())
                .test_value(),
            Value::Int(1)
        );

        assert_eq!(test_object(&engine, item).state.container, None);
        assert_eq!(
            test_object(&engine, item).state.contents_link_generation,
            generation_before + 1,
            "the removed container does not own the child's link history"
        );
        assert_eq!(
            engine.objects.len(),
            1,
            "the pending container was cancelled"
        );
    }

    #[test]
    fn assign_removal_preserves_contained_ocf_for_self_containment() {
        // AssignRemoval refreshes a containing object's OCF before clearing
        // the removed object's Contained pointer (C4Object.cpp:297-306). If a
        // denumerated save made the object its own container, those are the
        // same object and the final deleted cache therefore stays contained.
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "SELF",
            "Self container",
            "#strict 2\npublic func RemoveSelf() { return RemoveObject(); }\n",
        ));
        let object = engine.spawn_test_object(SpawnConfig::new("SELF"));
        let index = engine.test_object_index(object);
        engine.objects[index].state.container = Some(object);
        engine.objects[index].state.contents = vec![object];
        engine.objects[index].state.contents_link_generation = 1;
        engine.refresh_object_ocf(index);
        let contained_ocf = engine.objects[index].state.ocf;
        assert_eq!(contained_ocf & crate::ocf::NOT_CONTAINED, 0);

        assert_eq!(
            engine
                .call_object_function(index, "RemoveSelf", Vec::new())
                .test_value(),
            Value::Bool(true)
        );

        let object = test_object(&engine, object);
        assert_eq!(object.state.status, crate::ObjectStatus::Deleted);
        assert_eq!(object.state.container, None);
        assert!(object.state.contents.is_empty());
        assert_eq!(
            object.state.ocf, contained_ocf,
            "the post-unlink raw Contained clear does not run SetOCF"
        );
    }

    #[test]
    fn assign_removal_parent_ocf_precedes_later_velocity_write() {
        // Removing a contained object runs the parent's UpdateMass and SetOCF
        // synchronously before the caller resumes (C4Object.cpp:297-305).
        // A later SetXDir writes raw motion without another SetOCF
        // (C4Script.cpp:697-732), so the cached hit-speed bits stay clear.
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition(
            "CONT",
            "Container",
            "#strict 2\npublic func RemoveThenAccelerate(object child) { RemoveObject(child); SetXDir(2, 0, 1); return 1; }\n",
        ));
        engine.register_test_definition(test_definition("ITEM", "Item", "#strict 2\n"));
        let parent = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let child = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(parent));
        let parent_index = engine.test_object_index(parent);

        assert_eq!(
            engine
                .call_object_function(
                    parent_index,
                    "RemoveThenAccelerate",
                    vec![crate::compat::object_reference_value(child)],
                )
                .test_value(),
            Value::Int(1)
        );

        let parent = test_object(&engine, parent);
        assert_eq!(parent.fixed_velocity.x, itofix(2));
        assert_eq!(
            parent.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "the deferred child unlink must not move the parent's SetOCF after SetXDir"
        );
    }

    #[test]
    fn collect_exit_ocf_precedes_later_velocity_write() {
        // Collect's Collection callback may Exit the item. Exit refreshes its
        // OCF before the callback's following SetXDir, and Collect skips its
        // tail CopyMotion once the item is no longer contained
        // (C4Object.cpp:1532-1563,5709-5714; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        let mut collector = structure_definition(
            "COLL",
            "Collector",
            r#"#strict 2
protected func Collection(object item)
{
  Exit(item);
  SetXDir(20, item, 1);
}
public func CollectItem(object item) { return Collect(item); }
"#,
        );
        collector.set_collection_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(collector);
        engine.register_test_definition(structure_definition("OUTS", "Outside", "#strict 2\n"));
        let mut item_definition = test_definition("ITEM", "Item", "#strict 2\n");
        item_definition.set_category(crate::CATEGORY_OBJECT);
        item_definition.set_collectible(true);
        engine.register_test_definition(item_definition);

        let collector = engine.spawn_test_object(SpawnConfig::new("COLL"));
        let outside = engine.spawn_test_object(SpawnConfig::new("OUTS"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(outside));
        let collector_index = engine.test_object_index(collector);

        assert_eq!(
            engine
                .call_object_function(
                    collector_index,
                    "CollectItem",
                    vec![crate::compat::object_reference_value(item)],
                )
                .test_value(),
            Value::Bool(true)
        );

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, None);
        assert_eq!(item.fixed_velocity.x, itofix(20));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "the deferred final Exit must not move SetOCF after SetXDir"
        );
        assert!(test_snapshot(&engine, collector).contents.is_empty());
        assert!(test_snapshot(&engine, outside).contents.is_empty());
    }

    #[test]
    fn enter_ocf_and_motion_precede_later_velocity_write() {
        // Enter copies the container motion and calls SetOCF before the script
        // resumes. A following SetXDir must therefore win in raw motion while
        // leaving the contained-era cache untouched (C4Object.cpp:1598-1624;
        // C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            "#strict 2\npublic func EnterThenAccelerate(object target) { Enter(target); SetXDir(2, 0, 1); return 1; }\n",
        ));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));
        let item_index = engine.test_object_index(item);

        engine
            .call_object_function(
                item_index,
                "EnterThenAccelerate",
                vec![crate::compat::object_reference_value(container)],
            )
            .test_value();

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, Some(container));
        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0
        );
    }

    #[test]
    fn initialize_enter_ocf_precedes_later_velocity_write() {
        // NewObject's DoCon refreshes OCF before it calls Initialize. Enter
        // then refreshes the contained mask before the following raw SetXDir,
        // and no creation-tail SetOCF runs afterwards (C4Game.cpp:1115-1131;
        // C4Object.cpp:1428-1511,1598-1624; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            "#strict 2\nfunc Initialize() { Enter(FindObject(CONT)); SetXDir(2, 0, 1); return 1; }\n",
        ));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, Some(container));
        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "the spawn fold must not move DoCon's SetOCF after Initialize"
        );
    }

    #[test]
    fn initialize_raw_velocity_keeps_docon_ocf() {
        // DoCon's SetOCF precedes Initialize, while SetXDir is only a raw
        // xdir write. NewObject performs no later SetOCF, so the creation
        // result keeps the pre-write hit-speed mask (C4Game.cpp:1115-1131;
        // C4Object.cpp:1428-1511; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            "#strict 2\nfunc Initialize() { SetXDir(2, 0, 1); return 1; }\n",
        ));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));

        let item = test_object(&engine, item);
        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "Initialize's raw dir write must not trigger a creation-tail SetOCF"
        );
    }

    #[test]
    fn initialize_set_action_replaces_docon_ocf() {
        // SetAction refreshes OCF after selecting its action and before its
        // callbacks. NewObject performs no later SetOCF, so Initialize must
        // retain the disabled action's cache (C4Game.cpp:1115-1131;
        // C4Object.cpp:1428-1511,4111-4197).
        let mut engine = clonk_engine("#strict 2\n");
        let mut definition = test_definition(
            "ITEM",
            "Item",
            "#strict 2\nfunc Initialize() { SetAction(\"Disabled\"); return 1; }\n",
        );
        definition.set_category(crate::CATEGORY_LIVING);
        definition.configure_actions(
            Some("Idle".to_owned()),
            HashMap::from([(
                "Disabled".to_owned(),
                ActionSpec {
                    disabled: true,
                    ..ActionSpec::default()
                },
            )]),
        );
        engine.register_test_definition(definition);

        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_alive(true));

        let item = test_object(&engine, item);
        assert_eq!(item.state.action.name, "Disabled");
        assert_eq!(
            item.state.ocf & crate::ocf::FIGHT_READY,
            0,
            "Initialize's SetAction cache must supersede DoCon's cache"
        );
    }

    #[test]
    fn native_create_initialize_raw_velocity_keeps_docon_ocf() {
        // FnCreateObject runs NewObject's Construction/DoCon/Initialize
        // lifecycle synchronously. Its deferred Rust materialization must
        // retain the same pre-SetXDir cache (C4Game.cpp:1115-1131;
        // C4Object.cpp:1428-1511; C4Script.cpp:697-732,1886-1902).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            "#strict 2\nfunc Initialize() { SetXDir(2, 0, 1); return 1; }\n",
        ));
        engine.register_test_definition(test_definition(
            "MAKE",
            "Maker",
            "#strict 2\nfunc Make() { return CreateObject(ITEM, 0, 0, -1); }\n",
        ));
        let maker = engine.spawn_test_object(SpawnConfig::new("MAKE"));
        let maker_index = engine.test_object_index(maker);
        let created = engine
            .call_object_function(maker_index, "Make", Vec::new())
            .test_value();
        let Value::Object(created) = created else {
            panic!("CreateObject must return the new object")
        };
        let item = test_object(&engine, ObjectId::new(created));

        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "native creation must not add a materialization-time SetOCF"
        );
    }

    #[test]
    fn initialize_effect_enter_ocf_precedes_later_velocity_write() {
        // AddEffect starts an object effect synchronously before Initialize
        // returns. Its Enter SetOCF therefore precedes the following raw dir
        // write and must survive creation materialization (C4Effect.cpp:97-136;
        // C4Object.cpp:1428-1511,1598-1624; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
func Initialize() { AddEffect("Move", this(), 100, 0, this()); return 1; }
func FxMoveStart(object target, int number, int temporary)
{
    Enter(FindObject(CONT));
    SetXDir(2, 0, 1);
    return 1;
}
"#,
        ));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, Some(container));
        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0,
            "the effect's Enter cache must precede its raw dir write"
        );
    }

    #[test]
    fn construction_enter_ocf_is_refreshed_by_docon() {
        // Construction runs before NewObject's initial DoCon. Its SetOCF
        // therefore observes the later raw SetXDir when DoCon refreshes the
        // mask, unlike the same sequence in Initialize (C4Game.cpp:1115-1131;
        // C4Object.cpp:1428-1511,1598-1624; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
local construction_ocf, initialize_ocf;
func Construction()
{
    construction_ocf = GetOCF();
    Enter(FindObject(CONT));
    SetXDir(2, 0, 1);
    return 1;
}
func Initialize() { initialize_ocf = GetOCF(); return 1; }
"#,
        ));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(
            SpawnConfig::new("ITEM").with_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO)),
        );

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, Some(container));
        assert_eq!(item.fixed_velocity.x, itofix(2));
        let construction_ocf = match item.state.local_vars.get("construction_ocf") {
            Some(Value::Int(ocf)) => *ocf as u32,
            other => panic!("Construction must record an integer OCF, got {other:?}"),
        };
        let initialize_ocf = match item.state.local_vars.get("initialize_ocf") {
            Some(Value::Int(ocf)) => *ocf as u32,
            other => panic!("Initialize must record an integer OCF, got {other:?}"),
        };
        assert_eq!(
            construction_ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            crate::ocf::HIT_SPEED1 | crate::ocf::HIT_SPEED2,
            "Construction observes Init's OCF refresh"
        );
        assert_eq!(
            initialize_ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            crate::ocf::HIT_SPEED1 | crate::ocf::HIT_SPEED2,
            "Initialize observes DoCon's OCF refresh"
        );
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            crate::ocf::HIT_SPEED1 | crate::ocf::HIT_SPEED2,
            "DoCon must supersede Construction's earlier cached mask"
        );
    }

    #[test]
    fn construction_initial_ocf_excludes_unlinked_own_solid_mask() {
        // C4Game::NewObject runs C4Object::Init (including its SetOCF) before
        // adding the object to Game.Objects. Construction therefore sees the
        // air at its position, not its own not-yet-linked solid mask
        // (C4Game.cpp:1115-1126; C4Object.cpp:198-216).
        let mut engine = clonk_engine("#strict 2\n");
        let mut definition = test_definition(
            "ITEM",
            "Item",
            "#strict 2\nlocal construction_ocf; func Construction() { construction_ocf = GetOCF(); return 1; }\n",
        );
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine.register_test_definition(definition);
        engine.set_landscape(Landscape::flat(64, 32));

        let item =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(16, 16)));

        let item = test_object(&engine, item);
        let construction_ocf = match item.state.local_vars.get("construction_ocf") {
            Some(Value::Int(ocf)) => *ocf as u32,
            other => panic!("Construction must record an integer OCF, got {other:?}"),
        };
        assert_eq!(
            construction_ocf & crate::ocf::IN_SOLID,
            0,
            "Init must compute OCF before the newborn's mask joins object queries"
        );
    }

    #[test]
    fn initialize_sees_docon_solid_mask_put() {
        // Initial DoCon runs SetOCF first, then UpdateFace(true) puts the
        // completed solid mask before Completion and Initialize. Landscape
        // queries in Initialize therefore see that new vehicle pixel
        // (C4Object.cpp:1428-1511,5655-5690).
        let mut engine = clonk_engine("#strict 2\n");
        let mut definition = test_definition(
            "ITEM",
            "Item",
            "#strict 2\nlocal mask_solid; func Initialize() { mask_solid = GBackSolid(0, 0); return 1; }\n",
        );
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine.register_test_definition(definition);
        let grid = crate::landscape::PixelGrid::new(
            64,
            64,
            vec![0; 64 * 64],
            vec![0, 100, 100],
            vec![None, Some("Earth".to_owned()), Some("Vehicle".to_owned())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(64, vec![32; 64]).test_value();
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let item =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(16, 16)));

        assert_eq!(
            test_object(&engine, item)
                .state
                .local_vars
                .get("mask_solid"),
            Some(&Value::Bool(true)),
            "Initialize must query the mask put by DoCon's UpdateFace"
        );
    }

    #[test]
    fn construction_removal_skips_docon_initialize_and_mask_put() {
        // NewObject returns immediately when Construction clears Status, so
        // it runs neither initial DoCon nor Initialize and never puts the
        // full-con solid mask (C4Game.cpp:1121-1131; C4Object.cpp:240-313).
        let mut engine = clonk_engine("#strict 2\n");
        let mut definition = test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
local initialized;
func Construction() { RemoveObject(); return 1; }
func Initialize() { initialized = 1; return 1; }
"#,
        );
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        engine.register_test_definition(definition);
        let grid = crate::landscape::PixelGrid::new(
            64,
            64,
            vec![0; 64 * 64],
            vec![0, 100, 100],
            vec![None, Some("Earth".to_owned()), Some("Vehicle".to_owned())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(64, vec![32; 64]).test_value();
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let item =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(16, 16)));

        let item = test_object(&engine, item);
        assert!(item.destroyed);
        assert_ne!(
            item.state.local_vars.get("initialized"),
            Some(&Value::Int(1)),
            "Initialize must not run after Construction removal"
        );
        assert!(item.solid_mask_bake.is_none());
        assert!(item.solid_mask_instance_sequence.is_none());
        assert!(
            !engine
                .landscape()
                .test_value()
                .is_solid_at(item.state.position.x, item.state.position.y),
            "removed newborn must leave no vehicle pixel"
        );
    }

    #[test]
    fn inactive_newborn_does_not_block_its_docon_chop_probe() {
        // StatusDeactivate removes the newborn from Game.Objects but keeps a
        // nonzero status, so NewObject continues through DoCon. SetOCF's
        // AtObject probe cannot see the now-inactive object itself
        // (C4Game.cpp:1121-1131; C4GameObjects.cpp:54-70;
        // C4Object.cpp:549-575,1428-1511).
        let mut engine = clonk_engine("#strict 2\n");
        let mut definition = test_definition(
            "ITEM",
            "Item",
            r#"#strict 2
local initialize_ocf;
func Construction() { SetObjectStatus(2); return 1; }
func Initialize() { initialize_ocf = GetOCF(); return 1; }
"#,
        );
        definition.set_category(crate::CATEGORY_STATIC_BACK);
        definition.set_exclusive(true);
        definition.set_chopable(true);
        engine.register_test_definition(definition);

        let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));

        let item = test_object(&engine, item);
        assert_eq!(item.state.status, crate::ObjectStatus::Inactive);
        let initialize_ocf = match item.state.local_vars.get("initialize_ocf") {
            Some(Value::Int(ocf)) => *ocf as u32,
            other => panic!("Initialize must record an integer OCF, got {other:?}"),
        };
        assert_ne!(
            initialize_ocf & crate::ocf::CHOP,
            0,
            "inactive newborn must not appear in DoCon's main-list AtObject probe"
        );
    }

    #[test]
    fn exit_ocf_precedes_later_velocity_write() {
        // Exit writes its requested motion and calls SetOCF before returning;
        // a later SetXDir changes motion without refreshing that cache
        // (C4Object.cpp:1532-1563; C4Script.cpp:697-732).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(structure_definition("CONT", "Container", "#strict 2\n"));
        engine.register_test_definition(test_definition(
            "ITEM",
            "Item",
            "#strict 2\npublic func ExitThenAccelerate() { Exit(); SetXDir(2, 0, 1); return 1; }\n",
        ));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let item_index = engine.test_object_index(item);

        engine
            .call_object_function(item_index, "ExitThenAccelerate", Vec::new())
            .test_value();

        let item = test_object(&engine, item);
        assert_eq!(item.state.container, None);
        assert_eq!(item.fixed_velocity.x, itofix(2));
        assert_eq!(
            item.state.ocf
                & (crate::ocf::HIT_SPEED1
                    | crate::ocf::HIT_SPEED2
                    | crate::ocf::HIT_SPEED3
                    | crate::ocf::HIT_SPEED4),
            0
        );
    }

    #[test]
    fn scroll_replaces_link_while_shift_preserves_link_identity() {
        // FnScrollContents removes the first live object and Add(stNone)
        // allocates a fresh tail link (C4Script.cpp:1793-1804;
        // C4ObjectList.cpp:296-308,129-132,240-259). ShiftContents instead
        // rotates the existing link chain (C4ObjectList.cpp:815-833).
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition(
            "CONT",
            "Container",
            r#"#strict 2
public func Scroll() { return ScrollContents(); }
public func Shift() { return ShiftContents(); }
"#,
        );
        engine.register_test_definition(container);
        for id in ["ITMA", "ITMB", "ITMC"] {
            let mut definition = test_definition(id, id, "#strict 2\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            engine.register_test_definition(definition);
        }
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let a = engine.spawn_test_object(SpawnConfig::new("ITMA").with_container(container));
        let b = engine.spawn_test_object(SpawnConfig::new("ITMB").with_container(container));
        let c = engine.spawn_test_object(SpawnConfig::new("ITMC").with_container(container));
        let container_index = engine.test_object_index(container);
        assert_eq!(test_snapshot(&engine, container).contents, [c, b, a]);

        let c_generation = test_object(&engine, c).state.contents_link_generation;
        assert_eq!(
            engine
                .call_object_function(container_index, "Scroll", Vec::new())
                .test_value(),
            Value::Object(b.as_u64())
        );
        assert_eq!(test_snapshot(&engine, container).contents, [b, a, c]);
        assert_eq!(
            test_object(&engine, c).state.contents_link_generation,
            c_generation + 1,
            "ScrollContents creates a new tail link"
        );

        let generations =
            [a, b, c].map(|item| test_object(&engine, item).state.contents_link_generation);
        assert_eq!(
            engine
                .call_object_function(container_index, "Shift", Vec::new())
                .test_value(),
            Value::Bool(true)
        );
        assert_eq!(test_snapshot(&engine, container).contents, [a, c, b]);
        assert_eq!(
            [a, b, c].map(|item| { test_object(&engine, item).state.contents_link_generation }),
            generations,
            "ShiftContents retains every existing link"
        );
    }

    #[test]
    fn activate_refill_applies_each_registered_iterator_removal_immediately() {
        // Initial A,B,C. A::CalcValue removes A (registered iterators move
        // to B), inserts N before C, then removes B (iterators move to N).
        // The next GetNext increments N -> C, so C is the next row. A final
        // snapshot alone cannot reconstruct that temporal chain.
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let mut a_definition = test_definition(
            "ITMA",
            "A",
            r#"#strict 2
        local inserted, victim, mutate;
        protected func CalcValue(object pInBase)
        {
          if (mutate)
          {
            mutate = 0;
            Exit();
            inserted->Enter(pInBase);
            victim->Exit();
          }
          return 1;
        }
        "#,
        );
        a_definition.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(a_definition);
        for (id, name) in [("ITMB", "B"), ("ITMC", "C")] {
            let mut definition = test_definition(id, name, "#strict 2\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            engine.register_test_definition(definition);
        }

        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let c = engine.spawn_test_object(SpawnConfig::new("ITMC").with_container(container));
        let b = engine.spawn_test_object(SpawnConfig::new("ITMB").with_container(container));
        let a = engine.spawn_test_object(SpawnConfig::new("ITMA").with_container(container));
        let inserted = engine.spawn_test_object(SpawnConfig::new("ITMC"));
        set_second_picture_row(&mut engine, inserted);
        assert_eq!(test_snapshot(&engine, container).contents, [a, b, c]);
        test_object_mut(&mut engine, a).state.local_vars.extend([
            (
                "inserted".to_string(),
                compat::object_reference_value(inserted),
            ),
            ("victim".to_string(), compat::object_reference_value(b)),
            ("mutate".to_string(), Value::Int(1)),
        ]);

        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_activate_menu(crew_index, container_index)
            .test_value();

        assert_eq!(test_snapshot(&engine, container).contents, [inserted, c]);
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_picture_objects(&menu), [Some(a), Some(c)]);
    }

    #[test]
    fn activate_refill_tracks_assign_removal_at_the_parent_unlink() {
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let mut a_definition = test_definition(
            "ITMA",
            "A",
            r#"#strict 2
        local inserted, victim, mutate;
        protected func CalcValue(object pInBase)
        {
          if (mutate)
          {
            mutate = 0;
            var new_item = inserted, old_successor = victim;
            RemoveObject();
            new_item->Enter(pInBase);
            RemoveObject(old_successor);
          }
          return 1;
        }
        "#,
        );
        a_definition.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(a_definition);
        for (id, name) in [("ITMB", "B"), ("ITMC", "C")] {
            let mut definition = test_definition(id, name, "#strict 2\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            engine.register_test_definition(definition);
        }

        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let c = engine.spawn_test_object(SpawnConfig::new("ITMC").with_container(container));
        let b = engine.spawn_test_object(SpawnConfig::new("ITMB").with_container(container));
        let a = engine.spawn_test_object(SpawnConfig::new("ITMA").with_container(container));
        let inserted = engine.spawn_test_object(SpawnConfig::new("ITMC"));
        set_second_picture_row(&mut engine, inserted);
        test_object_mut(&mut engine, a).state.local_vars.extend([
            (
                "inserted".to_string(),
                compat::object_reference_value(inserted),
            ),
            ("victim".to_string(), compat::object_reference_value(b)),
            ("mutate".to_string(), Value::Int(1)),
        ]);

        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_activate_menu(crew_index, container_index)
            .test_value();

        assert_eq!(test_snapshot(&engine, container).contents, [inserted, c]);
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_picture_objects(&menu), [Some(a), Some(c)]);
    }

    #[test]
    fn continuing_activate_refill_preserves_non_item_menu_state() {
        // DoRefillInternal(C4MN_Activate) calls ClearItems(false), not Init:
        // script-mutated layout and the original caption/symbol survive the
        // periodic refill (C4ObjectMenu.cpp:170-203; C4Menu.cpp:975-988).
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        engine
            .open_activate_menu(crew_index, container_index)
            .test_value();
        {
            let menu = engine.objects[crew_index].state.menu.as_mut().test_value();
            menu.caption = "Callback caption".to_string();
            menu.symbol_id = "ITEM".to_string();
            menu.style = 2;
            menu.columns = 3;
            menu.lines = 4;
            menu.text_progressing = true;
        }

        let menu = open_activate_test_menu(&mut engine, crew, container);
        assert_eq!(menu.caption, "Callback caption");
        assert_eq!(menu.symbol_id, "ITEM");
        assert_eq!((menu.style, menu.columns, menu.lines), (2, 3, 4));
        assert!(menu.text_progressing);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].text_display_progress, 0);

        engine
            .initialize_activate_menu(crew_index, container_index)
            .test_value();
        let fresh = test_menu(&engine, crew);
        assert_eq!(fresh.caption, "Container is empty.");
        assert_eq!(fresh.symbol_id, "CONT");
        assert_eq!((fresh.style, fresh.columns, fresh.lines), (0, 5, 0));
        assert!(!fresh.text_progressing);
        assert_eq!(fresh.items.len(), 1);
        assert_eq!(fresh.items[0].text_display_progress, -1);
    }

    #[test]
    fn activate_refill_does_not_alias_script_extra_data_with_runtime_freeze_state() {
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let script = r#"#strict 2
local menu_owner, replace_menu;
protected func CalcValue(object pInBase)
{
  if (replace_menu)
  {
    replace_menu = 0;
    var marker = -2147483647;
    marker--;
    CreateMenu(ITEM, menu_owner, menu_owner, 0, "Replacement", marker, 0, 0, 77);
    AddMenuItem("Only row", "DoNothing", ITEM, menu_owner);
  }
  return 1;
}
"#;
        let mut item = test_definition("ITEM", "Item", script);
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        test_object_mut(&mut engine, item).state.local_vars.extend([
            (
                "menu_owner".to_string(),
                compat::object_reference_value(crew),
            ),
            ("replace_menu".to_string(), Value::Int(1)),
        ]);
        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);

        let menu = open_activate_test_menu(&mut engine, crew, container);
        // FnCreateMenu's idMenuID is a native C4ID. The integer 77 is
        // converted by FnCnvInt2Id before the body and remains C4ID-typed.
        assert_eq!(menu.identification, Value::C4Id("0077".into()));
        assert_eq!(menu.extra_data, i32::MIN);
        assert_eq!(menu.internal_refill_token, 0);
        assert_eq!(menu.refill_object, None);
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Only row");
    }

    #[test]
    fn activate_refill_continues_into_a_nested_internal_reinitialization() {
        let mut engine = clonk_engine("#strict 2\n");
        let container = structure_definition("CONT", "Container", "#strict 2\n");
        engine.register_test_definition(container);
        let mut first = test_definition(
            "ITMA",
            "A",
            r#"#strict 2
        static reopened;
        local menu_owner;
        protected func CalcValue(object pInBase)
        {
          if (!reopened)
          {
            reopened = 1;
            SetCommand(menu_owner, "Take");
            ExecuteCommand(menu_owner);
          }
          return 1;
        }
        "#,
        );
        first.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(first);
        let mut second = test_definition("ITMB", "B", "#strict 2\n");
        second.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(second);

        let crew = engine.spawn_test_object(SpawnConfig::new("CLNK"));
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        engine.spawn_test_object(SpawnConfig::new("ITMB").with_container(container));
        let first = engine.spawn_test_object(SpawnConfig::new("ITMA").with_container(container));
        contain_object(&mut engine, crew, container);
        test_object_mut(&mut engine, first).state.local_vars.insert(
            "menu_owner".to_string(),
            compat::object_reference_value(crew),
        );

        let crew_index = engine.test_object_index(crew);
        let container_index = engine.test_object_index(container);
        let menu = open_activate_test_menu(&mut engine, crew, container);
        assert_eq!(menu.identification, Value::Int(6));
        assert_eq!(
            menu_item_ids(&menu),
            ["ITMA", "ITMB", "CLNK", "ITMA", "ITMB", "CLNK"],
            "the nested Init rows survive and the suspended outer refill appends its current and remaining rows"
        );
        assert_eq!(menu.internal_refill_token, 0);
    }

    #[test]
    fn activate_target_reject_contents_force_closes_the_prior_menu() {
        let mut engine = clonk_engine("#strict\n");
        engine.register_test_definition(test_definition(
            "REJT",
            "Rejecting container",
            "#strict 2\nprotected func RejectContents() { return true; }\n",
        ));
        let crew = register_player_crew(&mut engine);
        let container = engine.spawn_test_object(SpawnConfig::new("REJT"));
        let crew_index = engine.test_object_index(crew);
        engine
            .open_context_menu(crew_index, crew_index, false, None)
            .test_value();
        assert!(engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .is_some());
        engine.objects[crew_index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Activate).with_target2(Some(container)),
        )]);

        engine.execute_object_command_now(crew).test_value();

        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        assert!(engine.pending_menu_requests.is_empty());
        assert!(test_snapshot(&engine, crew).command_stack.is_empty());
    }

    #[test]
    fn legacy_get_and_take_menus_use_cpp_ids_and_refill_target() {
        // Old-style C4CMD_Get Data=1 opens C4MN_Get (13), while C4CMD_Take
        // opens C4MN_Activate (6) against the actor's current container.
        // Both are internal engine menus; no serialized app request survives
        // (C4Command.cpp:1129-1135,1307-1315; C4ObjectMenu.h:29-49).
        let mut engine = clonk_engine("#strict 2\n");
        let mut container = structure_definition("CONT", "Container", "#strict 2\n");
        container.set_entrance_rect(Some(crate::DefinitionRect::new(-5, -5, 10, 10)));
        engine.register_test_definition(container);
        let mut cargo = test_definition("CRGO", "Cargo", "#strict 2\n");
        cargo.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(cargo);
        let crew = register_player_crew(&mut engine);
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.category = crate::CATEGORY_LIVING;
        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        engine.spawn_test_object(SpawnConfig::new("CRGO").with_container(container));
        contain_object(&mut engine, crew, container);

        engine.objects[crew_index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Get)
                .with_target(Some(container))
                .with_data(crate::CommandData::Integer(1)),
        )]);
        engine.execute_object_command_now(crew).test_value();
        let get_menu = test_menu(&engine, crew);
        assert_eq!(get_menu.identification, Value::Int(13));
        assert_eq!(get_menu.refill_object, Some(container));
        assert!(engine.pending_menu_requests.is_empty());

        engine.close_object_menu(crew, true).test_value();
        test_object_mut(&mut engine, crew).apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Take),
        )]);
        engine.execute_object_command_now(crew).test_value();
        let activate_menu = test_menu(&engine, crew);
        assert_eq!(activate_menu.identification, Value::Int(6));
        assert_eq!(activate_menu.refill_object, Some(container));
        assert!(engine.pending_menu_requests.is_empty());
    }

    #[test]
    fn contained_throw_puts_carried_item_into_container_immediately() {
        // ContainedControl executes C4CMD_Throw synchronously
        // (C4Object.cpp:3280-3282); C4Command::Throw delegates a contained
        // Clonk to ObjectComPutTake, which puts its first content into the
        // containing object (C4Command.cpp:966-970; C4ObjectCom.cpp:700-712).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict\nlocal put_called; protected func Put() { put_called = 1; return(1); }\n",
        );
        engine.register_test_script_definition("HUT1", "Hut", "#strict\n");
        engine.register_test_script_definition("FLAG", "Flag", "#strict\n");
        let crew = register_player_crew(&mut engine);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        contain_object(&mut engine, crew, hut);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let flag = test_snapshot(&engine, flag);
        assert_eq!(
            flag.container,
            Some(hut),
            "the carried flag is put into the hut before control returns"
        );
        assert_eq!(
            test_snapshot(&engine, crew).local_vars.get("put_called"),
            Some(&Value::Int(1)),
            "ObjectComPut callbacks complete before contained control returns"
        );
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "the synchronous Throw command has finished"
        );
    }

    #[test]
    fn old_contained_throw_runs_captured_function_after_put_exits_driver() {
        // Old definitions call the captured sf only after the synchronous
        // hardcoded Throw. Put may have exited the driver by then; C++ still
        // executes sf with a null object and the function owner's definition
        // context (C4Object.cpp:3284-3298; C4AulExec.cpp:330-359).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict\nprotected func Put() { Exit(); return(1); }\n",
        );
        let mut hut_definition = test_definition(
            "HUT1",
            "Hut",
            r#"
            #strict 2
            static late_calls, late_this;
            protected func ContainedThrow(object driver)
            {
                late_calls++;
                late_this = this();
                return true;
            }
            public func GetLateCalls() { return late_calls; }
            public func GetLateThis() { return late_this; }
        "#,
        );
        hut_definition.set_version([4, 9, 1, 2, 0]);
        engine.register_test_definition(hut_definition);
        engine.register_test_script_definition("FLAG", "Flag", "#strict\n");
        let crew = register_player_crew(&mut engine);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        contain_object(&mut engine, crew, hut);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        assert_eq!(
            test_snapshot(&engine, crew).container,
            None,
            "the synchronous Put callback exits the driver before sf->Exec"
        );
        assert_eq!(test_snapshot(&engine, flag).container, Some(hut));
        let hut_index = engine.test_object_index(hut);
        assert_eq!(
            engine
                .call_object_function(hut_index, "GetLateCalls", Vec::new())
                .expect("read definition static"),
            Value::Int(1),
            "the pinned late function runs in definition context with this == nil"
        );
        assert_eq!(
            engine
                .call_object_function(hut_index, "GetLateThis", Vec::new())
                .expect("read captured this"),
            Value::Nil,
            "sf->Exec receives the current null Contained pointer, not the old hut"
        );
    }

    #[test]
    fn old_contained_throw_keeps_captured_host_after_driver_changes_container() {
        // The C4AulFunc pointer comes from the original container, while the
        // late receiver is whatever `Contained` points to after hardcoded
        // controls. Helper/static lookup therefore stays on HUT1, but `this`
        // and object locals belong to HUT2 (C4Object.cpp:3231-3239,3293-3298).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            r#"#strict 2
                local move_target;
                protected func Put()
                {
                    if (move_target) Enter(move_target);
                    return true;
                }
            "#,
        );
        let mut source_definition = test_definition(
            "HUT1",
            "Source hut",
            r#"#strict 2
            static captured_host_calls;
            local receiver_calls, seen_receiver;
            private func MarkCapturedHost()
            {
                captured_host_calls += 10;
                return true;
            }
            protected func ContainedThrow(object driver)
            {
                MarkCapturedHost();
                captured_host_calls++;
                receiver_calls++;
                seen_receiver = this();
                return true;
            }
            public func GetCapturedHostCalls() { return captured_host_calls; }
        "#,
        );
        source_definition.set_version([4, 9, 1, 2, 0]);
        engine.register_test_definition(source_definition);
        engine.register_test_definition(test_definition(
            "HUT2",
            "Destination hut",
            r#"#strict 2
                    static wrong_host_calls;
                    local receiver_calls, seen_receiver;
                    private func MarkCapturedHost()
                    {
                        wrong_host_calls += 100;
                        return true;
                    }
                    protected func ContainedThrow(object driver)
                    {
                        wrong_host_calls += 1000;
                        return true;
                    }
                    public func GetWrongHostCalls() { return wrong_host_calls; }
                    public func GetReceiverCalls() { return receiver_calls; }
                    public func GetSeenReceiver() { return seen_receiver; }
                "#,
        ));
        engine.register_test_script_definition("FLAG", "Flag", "#strict\n");
        let crew = register_player_crew(&mut engine);
        let source = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let destination = engine.spawn_test_object(SpawnConfig::new("HUT2"));
        let flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        test_object_mut(&mut engine, crew).state.local_vars.insert(
            "move_target".to_string(),
            compat::object_reference_value(destination),
        );
        contain_object(&mut engine, crew, source);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        assert_eq!(test_snapshot(&engine, crew).container, Some(destination));
        assert_eq!(test_snapshot(&engine, flag).container, Some(source));
        let source_index = engine.test_object_index(source);
        assert_eq!(
            engine
                .call_object_function(source_index, "GetCapturedHostCalls", Vec::new())
                .expect("read captured static"),
            Value::Int(11),
            "the pinned body and its helper resolve on the captured HUT1 host"
        );
        let destination_index = engine.test_object_index(destination);
        assert_eq!(
            engine
                .call_object_function(destination_index, "GetWrongHostCalls", Vec::new())
                .expect("read destination static"),
            Value::Nil,
            "neither HUT2's same-named callback nor helper is resolved"
        );
        assert_eq!(
            engine
                .call_object_function(destination_index, "GetReceiverCalls", Vec::new())
                .expect("read receiver local"),
            Value::Int(1),
            "the captured body writes the current receiver's local cells"
        );
        assert_eq!(
            engine
                .call_object_function(destination_index, "GetSeenReceiver", Vec::new())
                .expect("read current this"),
            compat::object_reference_value(destination)
        );
    }

    /// Crew contained in a VehicleControl=Inside vehicle whose script is
    /// `vehicle_script`.
    fn inside_vehicle_fixture(engine: &mut Engine, vehicle_script: &str) -> (ObjectId, ObjectId) {
        inside_vehicle_fixture_with_clonk(engine, "#strict\n", vehicle_script)
    }

    fn inside_vehicle_fixture_with_clonk(
        engine: &mut Engine,
        clonk_script: &str,
        vehicle_script: &str,
    ) -> (ObjectId, ObjectId) {
        register_clonk(engine, "CLNK", clonk_script);
        let mut lorry = test_definition("LORY", "Lorry", vehicle_script);
        lorry.set_vehicle_control(crate::VEHICLE_CONTROL_INSIDE);
        engine.register_test_definition(lorry);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = spawn_crew(engine, "CLNK", 1);
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
        contain_object(engine, crew, lorry);
        (crew, lorry)
    }

    #[test]
    fn inside_vehicle_control_command_overloads_set_command() {
        // SetCommand's inside vehicle control overload (C4Object.cpp:
        // 3947-3961): a Contained def with C4D_VehicleControl_Inside gets
        // ControlCommand(name, target, tx, ty, target2, data, this) — the
        // CLONK rides in slot 7 — and a truthy return consumes the command.
        let vehicle = r#"
#strict
protected func ControlCommand(szCommand, pTarget, iTx, iTy, pTarget2, iData, pByObj) {
  if (pByObj) return(1);
  return(0);
}
"#;
        let mut engine = Engine::new();
        let (crew, lorry) = inside_vehicle_fixture(&mut engine, vehicle);

        engine.player_in_com(1, COM_DOWN, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the vehicle's ControlCommand consumed the Exit SetCommand"
        );
        assert_eq!(
            test_object(&engine, lorry).state.controller,
            1,
            "Contained->Controller = Controller (C4Object.cpp:3950)"
        );
    }

    #[test]
    fn inside_vehicle_falsy_control_command_keeps_the_exit() {
        // A falsy overload falls through to the hardcoded push
        // (C4Object.cpp:3976-3977).
        let vehicle = r#"
#strict
protected func ControlCommand() { return(0); }
"#;
        let mut engine = Engine::new();
        let (crew, _) = inside_vehicle_fixture(&mut engine, vehicle);

        engine.player_in_com(1, COM_DOWN, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.command_stack.command_names(), vec!["Exit"]);
    }

    #[test]
    fn outside_vehicle_control_command_overloads_pushed_set_command() {
        // The outside twin (C4Object.cpp:3962-3974): while pushing a
        // C4D_VehicleControl_Outside target, its ControlCommand (six args,
        // no clonk slot) may consume the Set command.
        let vehicle = r#"
#strict
protected func ControlCommand(szCommand) { return(1); }
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut lorry = test_definition("LORY", "Lorry", vehicle);
        lorry.set_vehicle_control(crate::VEHICLE_CONTROL_OUTSIDE);
        engine.register_test_definition(lorry);
        let crew = register_player_crew(&mut engine);
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine
            .player_object_command(1, CommandId::Exit, None, 0, 0)
            .test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the pushed vehicle's ControlCommand consumed the command"
        );
    }

    #[test]
    fn script_set_command_uses_inside_vehicle_overload_without_control_arms() {
        // FnSetCommand calls C4Object::SetCommand with fControl=false. The
        // contained VehicleControl=Inside arm is nevertheless unconditional,
        // while CloseMenu and the clonk's own ControlCommand stay disabled.
        let clonk = r#"
#strict
local own_calls;
protected func ControlCommand() { own_calls++; return(1); }
public func IssueExit() {
  CreateMenu(CLNK, this(), this(), 0, "Open");
  return SetCommand(this(), "Exit", 0, 0, 24, 0, 25);
}
"#;
        let vehicle = r#"
#strict
local calls, seen_command, seen_target, seen_tx, seen_ty, seen_target2, seen_data, seen_by;
protected func ControlCommand(command, target, tx, ty, target2, data, by) {
  calls++;
  seen_command = command;
  seen_target = target;
  seen_tx = tx;
  seen_ty = ty;
  seen_target2 = target2;
  seen_data = data;
  seen_by = by;
  return(1);
}
"#;
        let mut engine = Engine::new();
        let (crew, lorry) = inside_vehicle_fixture_with_clonk(&mut engine, clonk, vehicle);
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.controller = 17;

        let result = engine.call_test_object_function(crew_index, "IssueExit", Vec::new());

        assert_eq!(result, Value::Bool(true));
        let crew_index = engine.test_object_index(crew);
        assert!(
            engine.objects[crew_index].commands.is_empty(),
            "the inside callback consumes the command"
        );
        assert!(
            engine.objects[crew_index].state.menu.is_some(),
            "fControl=false must not close the clonk's menu"
        );
        assert_eq!(
            object_local(&engine, crew, "own_calls"),
            Some(&Value::Nil),
            "fControl=false must not call the clonk's own overload"
        );

        let lorry_index = engine.test_object_index(lorry);
        assert_eq!(engine.objects[lorry_index].state.controller, 17);
        let locals = &engine.objects[lorry_index].state.local_vars;
        assert_eq!(locals.get("calls"), Some(&Value::Int(1)));
        assert_eq!(
            locals.get("seen_command"),
            Some(&Value::String("Exit".to_string().into()))
        );
        assert_eq!(locals.get("seen_target"), Some(&Value::Nil));
        assert_eq!(
            locals.get("seen_tx"),
            Some(&Value::Nil),
            "FnSetCommand preserves its omitted Tx as a nil callback value"
        );
        assert_eq!(locals.get("seen_ty"), Some(&Value::Int(24)));
        assert_eq!(locals.get("seen_target2"), Some(&Value::Nil));
        assert_eq!(locals.get("seen_data"), Some(&Value::Int(25)));
        assert_eq!(locals.get("seen_by"), Some(&Value::Object(crew.as_u64())));
    }

    #[test]
    fn script_set_command_uses_outside_vehicle_overload_and_transfers_controller() {
        // The pushed VehicleControl=Outside twin receives exactly the six
        // regular arguments (no seventh clonk slot), inherits Controller,
        // and may consume a script SetCommand even though fControl is false.
        let clonk = r#"
#strict
public func IssueExit() { return SetCommand(this(), "Exit", 0, 0, 24, 0, 25); }
"#;
        let vehicle = r#"
#strict
local calls, seen_command, seen_target, seen_tx, seen_ty, seen_target2, seen_data, seen_seventh;
protected func ControlCommand(command, target, tx, ty, target2, data, seventh) {
  calls++;
  seen_command = command;
  seen_target = target;
  seen_tx = tx;
  seen_ty = ty;
  seen_target2 = target2;
  seen_data = data;
  seen_seventh = seventh;
  return(1);
}
"#;
        let mut engine = clonk_engine(clonk);
        let mut lorry = test_definition("LORY", "Lorry", vehicle);
        lorry.set_vehicle_control(crate::VEHICLE_CONTROL_OUTSIDE);
        engine.register_test_definition(lorry);
        let crew = register_player_crew(&mut engine);
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);
        engine.objects[crew_index].state.controller = 17;

        let result = engine.call_test_object_function(crew_index, "IssueExit", Vec::new());

        assert_eq!(result, Value::Bool(true));
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "the outside callback consumes the command"
        );
        let lorry_index = engine.test_object_index(lorry);
        assert_eq!(engine.objects[lorry_index].state.controller, 17);
        let locals = &engine.objects[lorry_index].state.local_vars;
        assert_eq!(locals.get("calls"), Some(&Value::Int(1)));
        assert_eq!(
            locals.get("seen_command"),
            Some(&Value::String("Exit".to_string().into()))
        );
        assert_eq!(locals.get("seen_target"), Some(&Value::Nil));
        assert_eq!(locals.get("seen_tx"), Some(&Value::Nil));
        assert_eq!(locals.get("seen_ty"), Some(&Value::Int(24)));
        assert_eq!(locals.get("seen_target2"), Some(&Value::Nil));
        assert_eq!(locals.get("seen_data"), Some(&Value::Int(25)));
        assert_eq!(
            locals.get("seen_seventh"),
            Some(&Value::Nil),
            "the outside overload receives only six arguments"
        );
    }

    /// Crew contained in a hut that is player `base`'s home base.
    fn contained_base_fixture(engine: &mut Engine, base: i32) -> (ObjectId, ObjectId) {
        contained_base_fixture_with_script(engine, base, "#strict\n")
    }

    fn contained_base_fixture_with_script(
        engine: &mut Engine,
        base: i32,
        base_script: &str,
    ) -> (ObjectId, ObjectId) {
        register_clonk(engine, "CLNK", "#strict\n");
        let hut_def = test_definition("HUT1", "Hut", base_script);
        engine.register_test_definition(hut_def);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        if base != 1 {
            engine.register_test_player(PlayerConfig::new(base, "Host"));
        }
        let crew = spawn_crew(engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = base;
        contain_object(engine, crew, hut);
        (crew, hut)
    }

    fn execute_buy_command(
        engine: &mut Engine,
        crew: ObjectId,
        base: ObjectId,
        definition_id: &str,
        count: i32,
    ) {
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Buy)
                .with_target(Some(base))
                .with_tx(Some(count))
                .with_data(CommandData::Text(definition_id.to_string())),
        )]);
        engine.execute_object_command_now(crew).test_value();
    }

    fn execute_sell_command(
        engine: &mut Engine,
        crew: ObjectId,
        base: ObjectId,
        definition_id: &str,
        count: i32,
    ) {
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Sell)
                .with_target(Some(base))
                .with_tx(Some(count))
                .with_data(CommandData::Text(definition_id.to_string())),
        )]);
        engine.execute_object_command_now(crew).test_value();
    }

    #[test]
    fn noncaptain_buy_and_sell_sync_from_the_mutating_team_member() {
        // SyncHomebaseMaterialToTeam copies the complete list from the player
        // whose Buy/Sell2Home just changed it. Only the separate join-time
        // FromTeam path copies from the team captain (C4Player.cpp:850-852,
        // 887-891,2335-2367).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 2);
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Team", 0).with_player_ids(vec![1, 2])
        ]);
        engine.set_player_team(1, Some(1)).test_value();
        engine.set_player_team(2, Some(1)).test_value();
        engine.set_team_home_base_rule(true);

        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_collectible(true);
        item.set_rebuyable(true);
        item.set_value(25);
        engine.register_test_definition(item);
        engine.set_player_wealth(2, 25).test_value();
        for player in [1, 2] {
            engine
                .player_mut(player)
                .test_value()
                .set_home_base_material_entries(vec![("ITEM".into(), 1)]);
        }

        execute_buy_command(&mut engine, crew, hut, "ITEM", 1);
        for player in [1, 2] {
            assert_eq!(
                engine
                    .player(player)
                    .expect("team member remains")
                    .home_base_material()
                    .get("ITEM"),
                Some(&0),
                "player 2's decrement must replace every teammate's list"
            );
        }
        let bought = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "ITEM" && object.status.is_active())
            .test_value()
            .id;

        // Isolate the reverse mutation: a rebuyable sale must create the
        // missing slot from player 2 and then fan that list out to player 1.
        for player in [1, 2] {
            engine
                .player_mut(player)
                .test_value()
                .set_home_base_material_entries(Vec::new());
        }
        execute_sell_command(&mut engine, crew, hut, "ITEM", 1);
        assert!(
            engine
                .object_snapshot(bought)
                .is_none_or(|object| !object.status.is_active()),
            "the purchased item was sold"
        );
        for player in [1, 2] {
            assert_eq!(
                engine
                    .player(player)
                    .expect("team member remains")
                    .home_base_material()
                    .get("ITEM"),
                Some(&1),
                "player 2's increment must replace every teammate's list"
            );
        }
    }

    #[test]
    fn non_rebuyable_sell_still_syncs_the_base_owners_material_list() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 2);
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Team", 0).with_player_ids(vec![1, 2])
        ]);
        engine.set_player_team(1, Some(1)).test_value();
        engine.set_player_team(2, Some(1)).test_value();
        engine.set_team_home_base_rule(true);

        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_value(5);
        engine.register_test_definition(item);
        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(vec![("OLD1".into(), 9)]);
        engine
            .player_mut(2)
            .test_value()
            .set_home_base_material_entries(vec![("KEEP".into(), 3)]);
        let sold = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));

        execute_sell_command(&mut engine, crew, hut, "ITEM", 1);

        assert!(
            engine
                .object_snapshot(sold)
                .is_none_or(|object| !object.status.is_active()),
            "the non-Rebuyable item is still sold"
        );
        assert_eq!(engine.player(2).expect("base owner remains").wealth(), 5);
        for player in [1, 2] {
            assert_eq!(
                engine
                    .player(player)
                    .expect("team member remains")
                    .home_base_material_entries(),
                &[("KEEP".into(), 3)],
                "a valid SellTo definition always runs team material sync"
            );
        }
    }

    #[test]
    fn explicit_buy_obeys_the_global_buy_gate() {
        // C4Command::Buy checks BASEFUNC_Buy before either its implicit or
        // explicit-target paths. In particular, an explicit target must not
        // turn an existing target content into a purchase when buying is
        // globally disabled.
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut lorry = test_definition("LORY", "Lorry", "#strict 2\n");
        lorry.set_collectible(true);
        lorry.set_value(25);
        engine.register_test_definition(lorry);
        engine.set_player_wealth(1, 25).test_value();
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .test_value();
        let existing = engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        engine.set_base_buy_enabled(false);

        execute_buy_command(&mut engine, crew, hut, "LORY", 1);

        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "the disabled Buy command fails and leaves the stack"
        );
        assert_eq!(engine.player(1).expect("player").wealth(), 25);
        assert_eq!(
            engine
                .player(1)
                .expect("player")
                .home_base_material()
                .get("LORY"),
            Some(&1)
        );
        assert_eq!(
            test_snapshot(&engine, existing).container,
            Some(hut),
            "an explicit target is not a content-transfer shortcut"
        );
        assert_eq!(
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "LORY" && object.status.is_active())
                .count(),
            1,
            "no purchase object is created"
        );
    }

    #[test]
    fn buy_outside_the_explicit_base_pushes_the_cpp_enter_subcommand() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut lorry = test_definition("LORY", "Lorry", "#strict 2\n");
        lorry.set_value(25);
        engine.register_test_definition(lorry);
        engine.set_player_wealth(1, 25).test_value();
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .test_value();
        engine
            .apply_object_update(
                crew,
                crate::ObjectUpdate::new()
                    .clear_container()
                    .with_position(Vector2::new(100, 0)),
            )
            .test_value();

        execute_buy_command(&mut engine, crew, hut, "LORY", 1);

        let stack = test_snapshot(&engine, crew).command_stack;
        assert_eq!(stack.command_names(), ["Enter", "Buy"]);
        let views = stack.command_views();
        assert_eq!(views[0].target, Some(hut));
        assert_eq!(views[1].target, Some(hut));
        let serialized = serde_json::to_value(&stack).test_value();
        let commands = serialized["commands"].as_array().test_value();
        assert_eq!(commands[0]["update_interval"], serde_json::json!(50));
        assert_eq!(commands[0]["mode"], serde_json::json!("SilentSub"));
        assert_eq!(engine.player(1).expect("player").wealth(), 25);
        assert_eq!(
            engine
                .player(1)
                .expect("player")
                .home_base_material()
                .get("LORY"),
            Some(&1),
            "entering the base precedes all purchase side effects"
        );
        assert!(engine
            .snapshot()
            .objects
            .iter()
            .all(|object| object.definition_id != "LORY"));
    }

    #[test]
    fn buy_refuses_hostile_and_eliminated_base_owners_without_economic_side_effects() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 2);
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_value(20);
        engine.register_test_definition(item);
        engine.set_player_wealth(2, 100).test_value();
        engine
            .set_player_home_base_material(2, HashMap::from([("ITEM".to_string(), 2)]))
            .test_value();
        let initial_objects = engine.snapshot().objects.len();

        let assert_unchanged =
            |engine: &Engine| {
                let owner = engine.player(2).test_value();
                assert_eq!(owner.wealth(), 100);
                assert_eq!(owner.home_base_material().get("ITEM"), Some(&2));
                assert_eq!(engine.snapshot().objects.len(), initial_objects);
                assert!(engine.snapshot().objects.iter().all(|object| {
                    object.definition_id != "ITEM" || !object.status.is_active()
                }));
            };

        engine.set_hostility(1, 2, true).test_value();
        execute_buy_command(&mut engine, crew, hut, "ITEM", 1);
        assert_unchanged(&engine);

        engine.set_hostility(1, 2, false).test_value();
        engine
            .set_player_status(2, PlayerStatus::Eliminated)
            .test_value();
        execute_buy_command(&mut engine, crew, hut, "ITEM", 1);
        assert_unchanged(&engine);
    }

    #[test]
    fn buy_command_recruits_crew_and_runs_both_price_hooks_before_purchase() {
        let mut engine = Engine::with_seed(0);
        let base_script = r#"#strict 2
protected func CalcBuyValue(id definition, int value)
{
    if (definition == CREW) return value + 3;
    return value;
}
"#;
        let (actor, hut) = contained_base_fixture_with_script(&mut engine, 2, base_script);
        let crew_script = r#"#strict 2
local order, purchase_wealth, purchase_stock, purchase_base;
protected func CalcDefValue(object base, int player)
{
    if (!base) return 1000;
    return 10 * player;
}
public func Recruitment(int player)
{
    order = player;
    return true;
}
public func Purchase(int player, object base)
{
    order = order * 10 + player;
    purchase_wealth = GetWealth(player);
    purchase_stock = GetHomebaseMaterial(player, CREW);
    purchase_base = base;
    return true;
}
"#;
        let mut recruit = test_definition("CREW", "Recruit", crew_script);
        recruit.set_value(99);
        recruit.set_category(crate::CATEGORY_LIVING);
        recruit.set_crew_member(true);
        engine.register_test_definition(recruit);
        engine.set_player_wealth(2, 100).test_value();
        engine
            .set_player_home_base_material(2, HashMap::from([("CREW".to_string(), 1)]))
            .test_value();
        engine.set_standard_names(Some("Twonky\n".to_owned()));
        let rng_count_before = engine.debug_rng_clone().count;

        execute_buy_command(&mut engine, actor, hut, "CREW", 1);

        assert_eq!(engine.player(2).expect("payer remains").wealth(), 77);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get("CREW"),
            Some(&0),
            "CalcDefValue(20) then CalcBuyValue(+3) is charged before Purchase"
        );
        let bought = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "CREW" && object.status.is_active())
            .test_value()
            .id;
        let snapshot = test_snapshot(&engine, bought);
        assert!(snapshot.crew_member);
        assert_eq!(snapshot.owner, 1);
        assert_eq!(snapshot.container, Some(hut));
        assert_eq!(snapshot.local_vars.get("order"), Some(&Value::Int(12)));
        assert_eq!(
            snapshot.local_vars.get("purchase_wealth"),
            Some(&Value::Int(77))
        );
        assert_eq!(
            snapshot.local_vars.get("purchase_stock"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            snapshot.local_vars.get("purchase_base"),
            Some(&Value::Object(hut.as_u64()))
        );
        assert!(
            engine
                .player(1)
                .expect("recipient remains")
                .crew()
                .contains(&bought),
            "native MakeCrewMember joins the recipient's crew"
        );
        assert_eq!(
            engine
                .crew_object_info(bought)
                .expect("native MakeCrewMember creates C4ObjectInfo before Recruitment")
                .name,
            "Twonky"
        );
        assert_eq!(
            engine.debug_rng_clone().count,
            rng_count_before + 1,
            "fresh crew info consumes the synchronized name draw"
        );
    }

    #[test]
    fn buy_command_purchase_removal_stops_after_the_committed_iteration() {
        let mut engine = Engine::new();
        let base_script = r#"#strict 2
local purchase_count, purchase_wealth, purchase_stock;
public func RecordPurchase(int wealth, int stock)
{
    purchase_count++;
    purchase_wealth = wealth;
    purchase_stock = stock;
    return true;
}
"#;
        let (actor, hut) = contained_base_fixture_with_script(&mut engine, 1, base_script);
        let item_script = r#"#strict 2
public func Purchase(int player, object base)
{
    base->RecordPurchase(GetWealth(player), GetHomebaseMaterial(player, ITEM));
    RemoveObject();
    return true;
}
"#;
        let mut item = test_definition("ITEM", "Item", item_script);
        item.set_value(10);
        engine.register_test_definition(item);
        engine.set_player_wealth(1, 30).test_value();
        engine
            .set_player_home_base_material(1, HashMap::from([("ITEM".to_string(), 2)]))
            .test_value();

        execute_buy_command(&mut engine, actor, hut, "ITEM", 2);

        assert_eq!(engine.player(1).expect("buyer remains").wealth(), 20);
        assert_eq!(
            engine
                .player(1)
                .expect("buyer remains")
                .home_base_material()
                .get("ITEM"),
            Some(&1),
            "the removed result is null, so the second Buy2Base iteration does not run"
        );
        let base = test_snapshot(&engine, hut);
        assert_eq!(base.local_vars.get("purchase_count"), Some(&Value::Int(1)));
        assert_eq!(
            base.local_vars.get("purchase_wealth"),
            Some(&Value::Int(20))
        );
        assert_eq!(base.local_vars.get("purchase_stock"), Some(&Value::Int(1)));
        assert!(
            engine
                .snapshot()
                .objects
                .iter()
                .all(|object| object.definition_id != "ITEM" || !object.status.is_active()),
            "Purchase removes the bought object before Buy can return it"
        );
    }

    #[test]
    fn buy_count_purchases_every_item_in_one_execute_command() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 2);
        let lorry_script = r#"#strict 2
local purchase_player, purchase_base, purchase_container;
public func CalcDefValue(object base, int player)
{
    if (!base) return 1000;
    return 5 + 10 * player;
}
public func Purchase(int player, object base)
{
    purchase_player = player;
    purchase_base = base;
    purchase_container = Contained();
    return 1;
}
"#;
        let mut lorry = test_definition("LORY", "Lorry", lorry_script);
        lorry.set_collectible(true);
        lorry.set_value(99);
        engine.register_test_definition(lorry);
        engine.set_player_wealth(2, 100).test_value();
        engine
            .set_player_home_base_material(2, HashMap::from([("LORY".to_string(), 4)]))
            .test_value();
        let existing = engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));

        execute_buy_command(&mut engine, crew, hut, "LORY", 3);

        let player = engine.player(2).test_value();
        assert_eq!(
            player.wealth(),
            25,
            "three purchases use the dynamic value, not the static 99"
        );
        assert_eq!(
            player.home_base_material().get("LORY"),
            Some(&1),
            "all three purchases consume stock in the same execution"
        );
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "Tx=3 completes in one ExecuteCommand"
        );
        assert_eq!(
            test_snapshot(&engine, existing).container,
            Some(hut),
            "Buy creates fresh objects instead of transferring base contents"
        );
        let bought = engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| {
                object.id != existing && object.definition_id == "LORY" && object.status.is_active()
            })
            .collect::<Vec<_>>();
        assert_eq!(bought.len(), 3);
        assert!(
            bought
                .iter()
                .all(|object| object.owner == 1 && object.container == Some(hut)),
            "each purchased object belongs to the buyer and enters the base"
        );
        assert!(
            bought.iter().all(|object| {
                object.local_vars.get("purchase_player") == Some(&Value::Int(2))
                    && object.local_vars.get("purchase_base") == Some(&Value::Object(hut.as_u64()))
                    && object.local_vars.get("purchase_container") == Some(&Value::Nil)
            }),
            "each fresh object receives its Purchase callback"
        );
    }

    #[test]
    fn sell_count_recurses_contents_and_uses_the_allied_base_transaction() {
        let mut engine = clonk_engine("#strict 2\n");
        let hut_script = r#"#strict 2
local sale_order, sale_players, lifecycle_order;
protected func CalcSellValue(object item, int value) { return value + 1; }
public func RecordSale(int marker, int player)
{
    sale_order = sale_order * 10 + marker;
    sale_players = sale_players * 10 + player;
    lifecycle_order = lifecycle_order * 10 + marker;
    return true;
}
public func RecordDestruction(int marker)
{
    lifecycle_order = lifecycle_order * 10 + marker + 2;
    return true;
}
"#;
        engine.register_test_script_definition("HUT1", "Hut", hut_script);
        engine.register_test_player(PlayerConfig::new(1, "Seller"));
        engine.register_test_player(PlayerConfig::new(2, "Base owner"));
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 2;
        contain_object(&mut engine, crew, hut);

        let sale_script = |marker: i32, value: i32| {
            format!(
                r#"#strict 2
local sale_base;
public func CalcValue(object base, int player)
{{
    sale_base = base;
    if (!base || player != 2) return 1000;
    return {value};
}}
public func SellTo(int player) {{ return RMAP; }}
public func Sale(int player) {{ return sale_base->RecordSale({marker}, player); }}
protected func Destruction() {{ return sale_base->RecordDestruction({marker}); }}
"#
            )
        };
        engine.register_test_script_definition("CHLD", "Child", &sale_script(1, 3));
        engine.register_test_script_definition("PARN", "Parent", &sale_script(2, 7));
        let mut remapped = test_definition("RMAP", "Remapped", "#strict 2\n");
        remapped.set_rebuyable(true);
        engine.register_test_definition(remapped);

        let parent1 = engine.spawn_test_object(SpawnConfig::new("PARN").with_container(hut));
        let child1 = engine.spawn_test_object(SpawnConfig::new("CHLD").with_container(parent1));
        let parent2 = engine.spawn_test_object(SpawnConfig::new("PARN").with_container(hut));
        let child2 = engine.spawn_test_object(SpawnConfig::new("CHLD").with_container(parent2));
        let frame = engine.frame();

        execute_sell_command(&mut engine, crew, hut, "PARN", 2);

        assert_eq!(engine.frame(), frame, "both sales finish in one execution");
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "Tx=2 completes the Sell command in one Execute"
        );
        assert_eq!(engine.player(1).expect("seller").wealth(), 0);
        assert_eq!(
            engine.player(2).expect("base owner").wealth(),
            24,
            "each child is valued before its parent with base CalcSellValue"
        );
        assert_eq!(
            engine
                .player(2)
                .expect("base owner")
                .home_base_material()
                .get("RMAP"),
            Some(&4),
            "SellTo and Rebuyable run once for every recursive sale"
        );
        let locals = &test_object(&engine, hut).state.local_vars;
        assert_eq!(locals.get("sale_order"), Some(&Value::Int(1212)));
        assert_eq!(locals.get("sale_players"), Some(&Value::Int(2222)));
        assert_eq!(
            locals.get("lifecycle_order"),
            Some(&Value::Int(13_241_324)),
            "each child and parent fires Sale immediately before Destruction"
        );
        for sold in [child1, parent1, child2, parent2] {
            assert!(
                engine.find_object_index(sold).is_none_or(|index| {
                    !engine.objects[index].state.status.is_active()
                        || engine.objects[index].destroyed
                }),
                "every recursive sale removes its object"
            );
        }
    }

    #[test]
    fn sell_refuses_no_sell_and_crew_member_roots() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut no_sell = test_definition("NOSL", "No sell", "#strict 2\n");
        no_sell.set_value(20);
        no_sell.set_rebuyable(true);
        no_sell.set_no_sell(-2);
        engine.register_test_definition(no_sell);
        let mut crew_item = test_definition("CRIT", "Crew item", "#strict 2\n");
        crew_item.set_value(30);
        crew_item.set_rebuyable(true);
        crew_item.set_crew_member(true);
        engine.register_test_definition(crew_item);
        let no_sell_object = engine.spawn_test_object(SpawnConfig::new("NOSL").with_container(hut));
        let crew_object = engine.spawn_test_object(
            SpawnConfig::new("CRIT")
                .with_container(hut)
                .with_alive(true)
                .with_crew_member(true),
        );

        execute_sell_command(&mut engine, crew, hut, "NOSL", 1);
        execute_sell_command(&mut engine, crew, hut, "CRIT", 1);

        for refused in [no_sell_object, crew_object] {
            let snapshot = test_snapshot(&engine, refused);
            assert!(snapshot.status.is_active());
            assert_eq!(snapshot.container, Some(hut));
        }
        assert_eq!(engine.player(1).expect("player").wealth(), 0);
        assert!(engine
            .player(1)
            .expect("player")
            .home_base_material()
            .is_empty());
        assert!(test_snapshot(&engine, crew).command_stack.is_empty());
    }

    #[test]
    fn sell_refuses_hostile_and_surrendered_base_owners_without_side_effects() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 2);
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_value(20);
        item.set_rebuyable(true);
        engine.register_test_definition(item);
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));

        let assert_unchanged = |engine: &Engine| {
            let item = test_snapshot(engine, item);
            assert!(item.status.is_active());
            assert_eq!(item.container, Some(hut));
            let owner = engine.player(2).test_value();
            assert_eq!(owner.wealth(), 0);
            assert!(owner.home_base_material().is_empty());
        };

        engine.set_hostility(1, 2, true).test_value();
        execute_sell_command(&mut engine, crew, hut, "ITEM", 1);
        assert_unchanged(&engine);

        engine.set_hostility(1, 2, false).test_value();
        engine.set_player_surrendered(2, true).test_value();
        execute_sell_command(&mut engine, crew, hut, "ITEM", 1);
        assert_unchanged(&engine);
    }

    #[test]
    fn auto_sell_checks_elimination_and_surrender_before_exiting_the_candidate() {
        let mut engine = Engine::new();
        engine.register_test_script_definition("HUT1", "Hut", "#strict 2\n");
        let mut gold = test_definition("GOLD", "Gold", "#strict 2\n");
        gold.set_base_auto_sell(true);
        engine.register_test_definition(gold);
        engine.register_test_player(
            PlayerConfig::new(1, "Eliminated").with_status(PlayerStatus::Eliminated),
        );
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        let gold = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(hut));

        engine.auto_sell_base_contents(hut_index, 1).test_value();

        let gold = test_snapshot(&engine, gold);
        assert!(gold.status.is_active());
        assert_eq!(gold.container, Some(hut));

        engine
            .set_player_status(1, PlayerStatus::Active)
            .test_value();
        engine.set_player_surrendered(1, true).test_value();
        engine.auto_sell_base_contents(hut_index, 1).test_value();

        let gold = test_snapshot(&engine, gold.id);
        assert!(gold.status.is_active());
        assert_eq!(gold.container, Some(hut));
    }

    #[test]
    fn script_execute_buy_is_visible_before_later_set_command() {
        let mut engine = Engine::new();
        let clonk_script = r#"#strict 2
local seen_wealth, seen_stock, seen_command;
public func BuyThenReplace(object base)
{
    SetCommand(this(), "Buy", base, 1, 0, 0, ITEM);
    ExecuteCommand();
    seen_wealth = GetWealth(1);
    seen_stock = GetHomebaseMaterial(1, ITEM);
    seen_command = GetCommand(this(), 0);
    return SetCommand(this(), "Wait", 0, 37);
}
"#;
        register_clonk(&mut engine, "CLNK", clonk_script);
        engine.register_test_script_definition("HUT1", "Hut", "#strict 2\n");
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_value(25);
        engine.register_test_definition(item);
        engine.register_test_player(PlayerConfig::new(1, "Buyer"));
        engine.set_player_wealth(1, 100).test_value();
        engine
            .set_player_home_base_material(1, HashMap::from([("ITEM".to_string(), 2)]))
            .test_value();
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        test_object_mut(&mut engine, hut).state.base = 1;
        contain_object(&mut engine, crew, hut);

        let crew_index = engine.test_object_index(crew);
        let result = engine.call_test_object_function(
            crew_index,
            "BuyThenReplace",
            vec![Value::Object(hut.as_u64())],
        );

        assert_eq!(result, Value::Bool(true));
        let crew_index = engine.test_object_index(crew);
        let locals = &engine.objects[crew_index].state.local_vars;
        assert_eq!(locals.get("seen_wealth"), Some(&Value::Int(75)));
        assert_eq!(locals.get("seen_stock"), Some(&Value::Int(1)));
        assert_eq!(locals.get("seen_command"), Some(&Value::Nil));
        let stack = engine.objects[crew_index].commands.snapshot();
        assert_eq!(stack.command_names(), ["Wait"]);
        assert_eq!(stack.command_views()[0].tx, Some(37));
        assert!(engine.snapshot().objects.iter().any(|object| {
            object.definition_id == "ITEM"
                && object.status.is_active()
                && object.owner == 1
                && object.container == Some(hut)
        }));
    }

    #[test]
    fn script_execute_sell_finishes_the_full_count_before_the_next_statement() {
        let mut engine = Engine::new();
        let clonk_script = r#"#strict 2
local seen_wealth, seen_stock, seen_command;
public func SellThenReplace(object base)
{
    SetCommand(this(), "Sell", base, 2, 0, 0, ITEM);
    ExecuteCommand();
    seen_wealth = GetWealth(2);
    seen_stock = GetHomebaseMaterial(2, ITEM);
    seen_command = GetCommand(this(), 0);
    return SetCommand(this(), "Wait", 0, 37);
}
"#;
        register_clonk(&mut engine, "CLNK", clonk_script);
        engine.register_test_script_definition("HUT1", "Hut", "#strict 2\n");
        let mut item = test_definition("ITEM", "Item", "#strict 2\n");
        item.set_value(5);
        item.set_rebuyable(true);
        engine.register_test_definition(item);
        engine.register_test_player(PlayerConfig::new(1, "Seller"));
        engine.register_test_player(PlayerConfig::new(2, "Base owner"));
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        test_object_mut(&mut engine, hut).state.base = 2;
        contain_object(&mut engine, crew, hut);
        let sold = [
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut)),
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut)),
        ];

        let crew_index = engine.test_object_index(crew);
        let result = engine.call_test_object_function(
            crew_index,
            "SellThenReplace",
            vec![Value::Object(hut.as_u64())],
        );

        assert_eq!(result, Value::Bool(true));
        let crew_index = engine.test_object_index(crew);
        let locals = &engine.objects[crew_index].state.local_vars;
        assert_eq!(locals.get("seen_wealth"), Some(&Value::Int(10)));
        assert_eq!(locals.get("seen_stock"), Some(&Value::Int(2)));
        assert_eq!(locals.get("seen_command"), Some(&Value::Nil));
        let stack = engine.objects[crew_index].commands.snapshot();
        assert_eq!(stack.command_names(), ["Wait"]);
        assert_eq!(stack.command_views()[0].tx, Some(37));
        assert!(sold.into_iter().all(|object| {
            engine.find_object_index(object).is_none_or(|index| {
                !engine.objects[index].state.status.is_active() || engine.objects[index].destroyed
            })
        }));
    }

    #[test]
    fn contained_com_up_opens_the_base_buy_menu() {
        // ContainedControl COM_Up (C4Object.cpp:3269-3274): a valid,
        // non-hostile base with BASEFUNC_Buy opens the buy menu on the
        // clonk (ActivateMenu(C4MN_Buy), pTarget = Contained).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);

        engine.player_in_com(1, COM_UP, 0).test_value();
        assert_eq!(
            test_menu(&engine, crew).identification,
            Value::Int(4),
            "COM_Up activates C4MN_Buy on the clonk"
        );
        assert!(
            engine.pending_menu_requests.is_empty(),
            "C4Object::ActivateMenu is engine-owned, not an app-side request"
        );
        assert_eq!(test_snapshot(&engine, crew).container, Some(hut));
    }

    #[test]
    fn contained_buy_menu_refills_from_the_base_players_material() {
        // C4Object::ActivateMenu(C4MN_Buy) creates a permanent menu on the
        // clonk (C4Object.cpp:1919-1930), and C4ObjectMenu::Refill adds the
        // base player's HomeBaseMaterial with its count, value and Buy
        // commands (C4ObjectMenu.cpp:207-237).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        test_object_mut(&mut engine, hut).state.owner = 7;
        let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
        lorry.set_value(25);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_test_definition(lorry);
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .test_value();

        engine.player_in_com(1, COM_UP, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(4), "C4MN_Buy");
        assert_eq!(
            menu.title_symbol,
            crate::ObjectMenuSymbol::Buy { owner: 7 },
            "C4Object::ActivateMenu composes C4MN_Buy with pTarget->Owner (C4Object.cpp:1919-1928; C4Menu.cpp:43-65)"
        );
        assert_eq!(
            menu.extra,
            crate::ObjectMenuExtra::Value,
            "C4MN_Buy enables C4MN_Extra_Value (C4Object.cpp:1926; C4Menu.cpp:843-907)"
        );
        assert!(menu.permanent);
        assert_eq!(menu.command_object, Some(crew));
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.len(), 1);
        let item = &menu.items[0];
        assert_eq!(item.caption, "Buy Lorry");
        assert_eq!(item.count, 1);
        assert_eq!(item.item_id, "LORY");
        assert_eq!(item.value, Some(25));
        assert_eq!(item.info_caption, "Carries cargo.");
        assert_eq!(
            item.command,
            format!(
                "AppendCommand(this,\"Buy\",Object({}),1,0,,0,LORY)&&ExecuteCommand()",
                hut.as_u64()
            )
        );
        assert_eq!(item.command2, item.command);
    }

    #[test]
    fn buy_menu_row_value_runs_definition_and_base_hooks_on_every_refill() {
        let mut engine = Engine::new();
        let base_script = r#"#strict 2
local buy_value_calls;
protected func CalcBuyValue(id definition, int value)
{
    if (definition != ITEM) return value;
    buy_value_calls = buy_value_calls + 1;
    return value + buy_value_calls;
}
"#;
        let (crew, hut) = contained_base_fixture_with_script(&mut engine, 2, base_script);
        let item_script = r#"#strict 2
protected func CalcDefValue(object base, int player)
{
    if (!base || player != 2) return 900;
    return 20;
}
"#;
        let mut item = test_definition("ITEM", "Item", item_script);
        item.set_value(99);
        engine.register_test_definition(item);
        engine
            .set_player_home_base_material(2, HashMap::from([("ITEM".to_string(), 1)]))
            .test_value();
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);

        engine
            .open_base_buy_menu(crew_index, hut_index)
            .test_value();
        assert_eq!(
            test_menu(&engine, crew).items[0].value,
            Some(21),
            "CalcDefValue sees the base player, then CalcBuyValue runs"
        );

        engine
            .refill_base_buy_menu(crew_index, hut_index)
            .test_value();
        assert_eq!(
            test_menu(&engine, crew).items[0].value,
            Some(22),
            "both hooks rerun on every refill"
        );
        assert_eq!(
            test_snapshot(&engine, hut)
                .local_vars
                .get("buy_value_calls"),
            Some(&Value::Int(2))
        );
    }

    #[test]
    fn periodic_refill_updates_buy_material_on_tick_35() {
        // HomeBaseMaterial is not part of RefillObject's contents count, so
        // C4Menu's common tick-35 pass is what makes an already-open Buy menu
        // observe external stock changes (C4Menu.cpp:990-999;
        // C4ObjectMenu.cpp:207-237,448-459).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        for (id, name) in [("LORY", "Lorry"), ("FLAG", "Flag")] {
            engine.register_test_script_definition(id, name, "#strict\n");
        }
        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(vec![("LORY".into(), 1)]);
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);
        engine
            .open_base_buy_menu(crew_index, hut_index)
            .test_value();
        // SetRefillObject's immediate refill leaves the contents-count cache
        // at zero in C++; prime its first Execute before changing material so
        // this regression isolates the periodic trigger.
        engine.execute_player_controls().test_value();

        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(vec![("LORY".into(), 4), ("FLAG".into(), 2)]);
        engine.frame = 34;
        engine.execute_player_controls().test_value();
        assert_eq!(engine.frame(), 34);
        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu_item_counts(&menu),
            vec![("LORY", 1)],
            "HomeBaseMaterial changes wait for the common periodic refill"
        );

        engine.frame = 35;
        engine.execute_player_controls().test_value();
        assert_eq!(engine.frame(), 35);
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_item_counts(&menu), vec![("LORY", 4), ("FLAG", 2)]);
    }

    #[test]
    fn contained_buy_menu_preserves_cpp_home_base_list_order_and_zero_rows() {
        let mut engine = Engine::new();
        let (crew, _) = contained_base_fixture(&mut engine, 1);
        for (id, name) in [("ZINC", "Zinc"), ("BRIK", "Brick")] {
            engine.register_test_script_definition(id, name, "#strict\n");
        }
        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(vec![("ZINC".into(), 2), ("BRIK".into(), 0)]);

        engine.player_in_com(1, COM_UP, 0).test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_item_counts(&menu), vec![("ZINC", 2), ("BRIK", 0)]);
    }

    #[test]
    fn contained_buy_menu_appends_new_rebuyable_stock_at_its_numeric_slot() {
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        for (id, name) in [("ZINC", "Zinc"), ("BRIK", "Brick"), ("AARD", "Aardvark")] {
            let mut definition = test_definition(id, name, "#strict\n");
            definition.set_value(1);
            if id == "AARD" {
                definition.set_rebuyable(true);
            }
            engine.register_test_definition(definition);
        }
        engine
            .player_mut(1)
            .test_value()
            .set_home_base_material_entries(vec![("ZINC".into(), 1), ("BRIK".into(), 1)]);

        let sold = engine.spawn_test_object(SpawnConfig::new("AARD").with_container(hut));
        engine.sell_object_to_home(sold, sold, 1).test_value();
        assert_eq!(
            engine
                .player(1)
                .expect("base player")
                .home_base_material_entries(),
            &[
                ("ZINC".to_string(), 1),
                ("BRIK".to_string(), 1),
                ("AARD".to_string(), 1),
            ],
            "Sell2Home appends a missing Rebuyable ID instead of sorting it"
        );

        engine.player_in_com(1, COM_UP, 0).test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu_item_ids(&menu), vec!["ZINC", "BRIK", "AARD"]);

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        assert_eq!(test_menu(&engine, crew).selection, 2);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 2);
        assert_eq!(
            menu_item_counts(&menu),
            vec![("ZINC", 1), ("BRIK", 1), ("AARD", 0)]
        );
        assert_eq!(engine.player(1).expect("base player").wealth(), 0);
        assert!(engine.snapshot().objects.iter().any(|object| {
            object.definition_id == "AARD"
                && object.status.is_active()
                && object.container == Some(hut)
        }));
    }

    #[test]
    fn contained_buy_menu_enter_purchases_and_refills() {
        // C4Player::InCom converts Throw to MenuEnter while a menu is open
        // (C4Player.cpp:1502-1513; C4Menu.cpp:1051-1057). The Buy row then
        // queues and executes C4CMD_Buy against Target->Base, consuming its
        // stock and the buyer's wealth (C4Command.cpp:2005-2035), while the
        // permanent menu refills (C4ObjectMenu.cpp:124-129,207-237).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
        lorry.set_value(25);
        engine.register_test_definition(lorry);
        engine.set_player_wealth(1, 25).test_value();
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .test_value();

        engine.player_in_com(1, COM_UP, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();

        let player = engine.player(1).test_value();
        assert_eq!(
            player.wealth(),
            0,
            "post-enter command stack: {:?}",
            test_snapshot(&engine, crew).command_stack.command_names()
        );
        assert_eq!(
            player.home_base_material().get("LORY"),
            Some(&0),
            "C4Player::Buy leaves the C4IDList entry at zero"
        );
        let snapshot = engine.snapshot();
        let bought = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "LORY" && object.status.is_active())
            .test_value();
        assert_eq!(bought.owner, 1);
        assert_eq!(bought.container, Some(hut));
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(4));
        assert_eq!(menu.items.len(), 1, "zero-count IDs remain visible");
        assert_eq!(menu.items[0].item_id, "LORY");
        assert_eq!(menu.items[0].count, 0);
        assert_eq!(menu.selection, 0);
    }

    #[test]
    fn contained_buy_menu_refill_preserves_the_numeric_selection() {
        // C4ObjectMenu::DoRefillInternal uses ClearItems(false), so the Buy
        // menu keeps its numeric selection while stock is rebuilt. The outer
        // C4Menu::RefillInternal then only adjusts it if that slot stopped
        // being selectable (C4ObjectMenu.cpp:207-237; C4Menu.cpp:947-988,
        // 1014-1038).
        let mut engine = Engine::new();
        let (crew, _hut) = contained_base_fixture(&mut engine, 1);
        for (id, name) in [("FLAG", "Flag"), ("LORY", "Lorry")] {
            let mut definition = test_definition(id, name, "#strict\n");
            definition.set_value(1);
            engine.register_test_definition(definition);
        }
        engine.set_player_wealth(1, 2).test_value();
        engine
            .set_player_home_base_material(
                1,
                HashMap::from([("FLAG".to_string(), 1), ("LORY".to_string(), 1)]),
            )
            .test_value();

        engine.player_in_com(1, COM_UP, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        assert_eq!(test_menu(&engine, crew).selection, 1);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "LORY");
        assert_eq!(menu.items[1].count, 0);
    }

    #[test]
    fn player_execute_opens_the_contained_buildings_auto_context_menu() {
        // C4Player::Execute calls Cursor->AutoContextMenu after controls
        // (C4Player.cpp:206-212). A crew member inside an opted-in building
        // with the player's preference enabled gets a permanent C4MN_Context
        // menu populated in Contents/Buy/Sell/Exit order
        // (C4Object.cpp:2044-2062; C4ObjectMenu.cpp:328-435).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.category = crate::CATEGORY_LIVING;
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        test_object_mut(&mut engine, hut).state.base = 1;
        contain_object(&mut engine, crew, hut);

        engine.execute_player_controls().test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(14), "C4MN_Context");
        assert_eq!(menu.style, 1, "C4MN_Style_Context");
        assert!(menu.permanent);
        assert!(!menu.user_menu);
        assert_eq!(menu.command_object, Some(crew));
        assert_eq!(menu.columns, 1);
        assert_eq!(
            menu_captions(&menu),
            vec!["Contents", "Buy", "Sell", "Exit"]
        );
    }

    #[test]
    fn contained_context_runs_script_declared_context_function() {
        // C4MN_Context inserts target `Context*` functions between the base
        // rows and Info/Exit. Their leading description block supplies the
        // caption/image/condition, and Enter executes ProtectedCall on the
        // target (C4ObjectMenu.cpp:398-399,670-682;
        // C4AulParse.cpp:309-380). This is WRKS::ContextConstruction's real
        // Tutorial07 path, reduced to one deterministic menu callback.
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(
            &mut engine,
            "WRKS",
            "Workshop",
            r#"
        #strict 2
        public func ContextConstruction(caller) {
            [Production|Image=CXCN|Condition=IsBuilt|Desc=Build a vehicle.]
            return CreateMenu(CXCN, caller, this(), 1, "No knowledge");
        }
        protected func IsBuilt() { return GetCon() >= 100; }
        "#,
        );
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let workshop = engine.spawn_test_object(SpawnConfig::new("WRKS"));
        contain_object(&mut engine, crew, workshop);

        engine.execute_player_controls().test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu_captions(&menu), vec!["Contents", "Production", "Exit"]);
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(
            test_menu(&engine, crew).identification,
            Value::C4Id("CXCN".to_owned())
        );
    }

    #[test]
    fn native_context_conditions_observe_and_extend_the_live_menu() {
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "TARG",
            "Target",
            r#"
                #strict 2
                func ContextReady(menu) {
                    [Native|Condition=MenuReady]
                    return 1;
                }
                func MenuReady(menu, image) {
                    if (GetMenu(menu) != 14) return false;
                    AddMenuItem("Injected", "", NONE, menu);
                    return true;
                }
                func ContextMissingCondition(menu) {
                    [Missing|Condition=DoesNotExist]
                    return 1;
                }
                "#,
        ));
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu.identification, Value::Int(14));
        assert_eq!(menu_captions(&menu), ["Missing", "Injected", "Native"]);
    }

    #[test]
    fn periodic_refill_rechecks_context_conditions_on_tick_35() {
        // Context rows are reconstructed by the common periodic refill, so
        // their script conditions run again even when the target's contents
        // count did not change (C4ObjectMenu.cpp:328-435;
        // C4Menu.cpp:990-999).
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "TARG",
            "Target",
            r#"
                #strict 2
                local enabled, condition_calls;
                func ContextDynamic(menu) {
                    [Dynamic|Condition=ShowDynamic]
                    return 1;
                }
                func ShowDynamic(menu, image) {
                    condition_calls++;
                    return enabled;
                }
                func Enable() {
                    enabled = true;
                    return true;
                }
                "#,
        ));
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu.identification, Value::Int(14));
        assert!(!menu.permanent, "mouse-style Context remains nonpermanent");
        assert!(menu.items.is_empty());
        let target_index = engine.test_object_index(target);
        assert_eq!(
            object_local(&engine, target, "condition_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine
                .call_object_function(target_index, "Enable", Vec::new())
                .expect("enable condition"),
            Value::Bool(true)
        );

        for _ in 0..34 {
            engine.tick_without_snapshot().test_value();
        }
        assert_eq!(engine.frame(), 34);
        let menu = test_menu(&engine, crew);
        assert!(menu.items.is_empty());
        let target_index = engine.test_object_index(target);
        assert_eq!(
            object_local(&engine, target, "condition_calls"),
            Some(&Value::Int(1)),
            "the condition is not polled on ordinary frames"
        );

        engine.tick_without_snapshot().test_value();
        assert_eq!(engine.frame(), 35);
        let menu = test_menu(&engine, crew);
        assert!(!menu.permanent);
        assert_eq!(menu_captions(&menu), ["Dynamic"]);
        let target_index = engine.test_object_index(target);
        assert_eq!(
            object_local(&engine, target, "condition_calls"),
            Some(&Value::Int(2))
        );
    }

    #[test]
    fn periodic_context_refill_preserves_location_and_live_shell() {
        // DoRefillInternal clears only the rows. The old selection and every
        // other menu property stay live while conditions run; a condition's
        // own SelectMenuItem then feeds the final AdjustSelection pass
        // (C4ObjectMenu.cpp:328-435; C4Menu.cpp:947-999).
        let mut engine = clonk_engine("#strict 2\n");
        let mut target_definition = structure_definition(
            "TARG",
            "Target",
            r#"
        #strict 2
        local enabled, condition_calls, seen_selection;
        func ContextDynamic(menu) {
            [Dynamic|Condition=ShowDynamic]
            return 1;
        }
        func ShowDynamic(menu, image) {
            condition_calls++;
            seen_selection = GetMenuSelection(menu);
            if (enabled) SelectMenuItem(0, menu);
            return enabled;
        }
        func Enable() {
            enabled = true;
            return true;
        }
        "#,
        );
        target_definition.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(target_definition);
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let target_index = engine.test_object_index(target);
        engine.objects[target_index].state.base = 1;
        contain_object(&mut engine, crew, target);
        let _ = open_native_context(&mut engine, crew, target);
        let target_index = engine.test_object_index(target);
        assert_eq!(
            object_local(&engine, target, "seen_selection"),
            Some(&Value::Int(-1)),
            "SetRefillObject's immediate refill is frozen too"
        );
        engine.execute_player_controls().test_value();

        let menu = test_object_mut(&mut engine, crew)
            .state
            .menu
            .as_mut()
            .test_value();
        assert_eq!(menu_captions(menu), ["Contents", "Buy", "Sell", "Exit"]);
        menu.selection = -1;
        menu.caption = "Script-mutated caption".to_string();
        menu.columns = 3;
        menu.lines = 2;
        menu.text_progressing = true;
        menu.location = Some(Vector2::new(17, 23));
        let runtime_id = menu.runtime_id;
        assert_ne!(runtime_id, 0);
        let target_index = engine.test_object_index(target);
        assert_eq!(
            engine
                .call_object_function(target_index, "Enable", Vec::new())
                .expect("enable condition"),
            Value::Bool(true)
        );

        engine.frame = 34;
        engine.execute_player_controls().test_value();
        let target_index = engine.test_object_index(target);
        assert_eq!(
            object_local(&engine, target, "condition_calls"),
            Some(&Value::Int(2))
        );

        engine.frame = 35;
        engine.execute_player_controls().test_value();
        let locals = &test_object(&engine, target).state.local_vars;
        assert_eq!(locals.get("condition_calls"), Some(&Value::Int(3)));
        assert_eq!(
            locals.get("seen_selection"),
            Some(&Value::Int(-1)),
            "the frozen refill does not auto-select its first native row"
        );
        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu.selection, 0,
            "callback selection survives AdjustSelection"
        );
        assert_eq!(menu.caption, "Script-mutated caption");
        assert_eq!((menu.columns, menu.lines), (3, 2));
        assert!(menu.text_progressing);
        assert!(!menu.permanent);
        assert_eq!(menu.location, Some(Vector2::new(17, 23)));
        assert_eq!(menu.runtime_id, runtime_id);
    }

    #[test]
    fn native_context_effect_walk_survives_current_effect_removal() {
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "EHST",
            "Effect host",
            r#"
                #strict 2
                func FxFirstContextOpen(target, number, menu, image) {
                    [First|Condition=DropCurrent]
                    return 1;
                }
                func DropCurrent(target, number, menu, image) {
                    RemoveEffect(0, target, number);
                    return true;
                }
                func FxSecondContextOpen(target, number, menu, image) {
                    [Second]
                    return 1;
                }
                "#,
        ));
        engine.register_test_script_definition("TARG", "Target", "#strict 2\n");
        let crew = register_player_crew(&mut engine);
        let host = engine.spawn_test_object(SpawnConfig::new("EHST"));
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let mut first = crate::EffectState::new("First");
        first.number = 2;
        first.priority = 100;
        first.command_target = Some(host.as_u64() as i32);
        let mut second = crate::EffectState::new("Second");
        second.number = 1;
        second.priority = 100;
        second.command_target = Some(host.as_u64() as i32);
        test_object_mut(&mut engine, target).state.effects = vec![first, second];

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu_captions(&menu), ["First", "Second"]);
        assert_eq!(
            menu.items[0].command,
            format!(
                "ProtectedCall(Object({}),\"FxFirstContextOpen\",Object({}),2,Object({}),NONE)",
                host.as_u64(),
                target.as_u64(),
                crew.as_u64()
            )
        );
    }

    #[test]
    fn native_context_effect_command_falls_back_when_condition_deletes_host() {
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "LHST",
            "Live host",
            r#"
                #strict 2
                func FxLiveContextOpen(target, number, menu, image) {
                    [Fallback|Condition=DeleteHost]
                    return 1;
                }
                func DeleteHost(target, number, menu, image) {
                    RemoveObject(this());
                    return true;
                }
                "#,
        ));
        engine.register_test_script_definition("TARG", "Target", "#strict 2\n");
        let crew = register_player_crew(&mut engine);
        let host = engine.spawn_test_object(SpawnConfig::new("LHST"));
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let mut effect = crate::EffectState::new("Live");
        effect.number = 4;
        effect.command_target = Some(host.as_u64() as i32);
        test_object_mut(&mut engine, target).state.effects = vec![effect];

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(
            menu.items[0].command,
            format!(
                "DefinitionCall(LHST, \"FxLiveContextOpen\", Object({}),4,Object({}),NONE)",
                target.as_u64(),
                crew.as_u64()
            )
        );
        assert!(engine.objects[engine.find_object_index(host).expect("host slot")].destroyed);
    }

    #[test]
    fn linked_context_menu_functions_preserve_cpp_func_list_order() {
        fn layer_functions(
            prefix: &str,
            parameters: &str,
            class: &str,
            function_layer: &str,
            caption_layer: &str,
        ) -> String {
            format!(
                r#"
func {prefix}{function_layer}First({parameters}) {{ [{class} {caption_layer} first] return 1; }}
func {prefix}Shared({parameters}) {{ [{class} {caption_layer} shared] return 1; }}
func {prefix}{function_layer}Last({parameters}) {{ [{class} {caption_layer} last] return 1; }}
"#,
            )
        }

        fn expected_linked_rows(class: &str) -> Vec<String> {
            [
                format!("{class} append last"),
                format!("{class} append shared"),
                format!("{class} append first"),
                format!("{class} local last"),
                format!("{class} local first"),
                format!("{class} include last"),
                format!("{class} include first"),
            ]
            .into_iter()
            .collect()
        }

        // C4AulScript::AppendTo copies appends at FuncL and includes at
        // Func0. GetSFunc then walks FuncL backwards, skipping every node
        // overloaded by a later same-name function. Exercise that one linked
        // order through each AddContextFunctions class, rather than merely
        // checking a declaration-only projection (C4AulLink.cpp:113-141;
        // C4Aul.cpp:357-379; C4ObjectMenu.cpp:558-685).
        let classes = [
            ("ActionContext", "menu, image, target", "Action"),
            ("FxGlowContext", "target, number, menu, image", "Effect"),
            ("AttachContext", "menu, image, target", "Attach"),
            ("Context", "menu", "Context"),
        ];

        let mut include_source = "#strict 2\n".to_owned();
        let mut append_source =
            "#strict 2\n#appendto AHST\n#appendto EHST\n#appendto ATCH\n#appendto TARG\n"
                .to_owned();
        for (prefix, parameters, class) in classes {
            include_source.push_str(&layer_functions(
                prefix, parameters, class, "Include", "include",
            ));
            append_source.push_str(&layer_functions(
                prefix, parameters, class, "Append", "append",
            ));
        }

        let global_early = r#"
#strict 2
global func FxWorldContextEarlyFirst(target, number, menu, image) { [Global early first] return 1; }
global func FxWorldContextShared(target, number, menu, image) { [Global early shared] return 1; }
global func FxWorldContextEarlyLast(target, number, menu, image) { [Global early last] return 1; }
"#;
        let global_late = r#"
#strict 2
global func FxWorldContextLateFirst(target, number, menu, image) { [Global late first] return 1; }
global func FxWorldContextShared(target, number, menu, image) { [Global late shared] return 1; }
global func FxWorldContextLateLast(target, number, menu, image) { [Global late last] return 1; }
"#;

        let mut engine = Engine::new();
        assert_eq!(
            engine.install_global_scripts(&[
                ("System.c4g/Early.c".to_owned(), global_early.to_owned()),
                ("System.c4g/Late.c".to_owned(), global_late.to_owned()),
            ]),
            2
        );
        register_clonk(&mut engine, "CLNK", "#strict 2\n");
        engine.register_test_script_definition("INCL", "Include", &include_source);

        let action_source = format!(
            "#strict 2\n#include INCL\n{}",
            layer_functions(
                "ActionContext",
                "menu, image, target",
                "Action",
                "Local",
                "local",
            )
        );
        engine.register_test_script_definition("AHST", "Action host", &action_source);

        let effect_source = format!(
            "#strict 2\n#include INCL\n{}",
            layer_functions(
                "FxGlowContext",
                "target, number, menu, image",
                "Effect",
                "Local",
                "local",
            )
        );
        engine.register_test_script_definition("EHST", "Effect host", &effect_source);

        let attached_source = format!(
            "#strict 2\n#include INCL\n{}",
            layer_functions(
                "AttachContext",
                "menu, image, target",
                "Attach",
                "Local",
                "local",
            )
        );
        let mut attached_definition = test_definition("ATCH", "Attachment", &attached_source);
        attached_definition.configure_actions(
            None,
            HashMap::from([(
                "Attached".to_owned(),
                ActionSpec::default().with_procedure("attach"),
            )]),
        );
        engine.register_test_definition(attached_definition);

        let target_source = format!(
            "#strict 2\n#include INCL\n{}",
            layer_functions("Context", "menu", "Context", "Local", "local")
        );
        let mut target_definition = test_definition("TARG", "Target", &target_source);
        target_definition.configure_actions(
            None,
            HashMap::from([("Use".to_owned(), ActionSpec::default())]),
        );
        engine.register_test_definition(target_definition);
        engine.register_test_script_definition("APND", "Appender", &append_source);
        engine.relink_scripts().test_value();

        let crew = register_player_crew(&mut engine);
        let action_host = engine.spawn_test_object(SpawnConfig::new("AHST"));
        let effect_host = engine.spawn_test_object(SpawnConfig::new("EHST"));
        let mut target_action = ActionState::new("Use");
        target_action.target = Some(action_host);
        let target = engine.spawn_test_object(
            SpawnConfig::new("TARG")
                .with_container(crew)
                .with_action(target_action),
        );
        let mut glow = crate::EffectState::new("Glow");
        glow.number = 7;
        glow.command_target = Some(effect_host.as_u64() as i32);
        let mut world = crate::EffectState::new("World");
        world.number = 11;
        test_object_mut(&mut engine, target).state.effects = vec![glow, world];
        let mut attached_action = ActionState::new("Attached");
        attached_action.target = Some(target);
        engine.spawn_test_object(SpawnConfig::new("ATCH").with_action(attached_action));

        let menu = open_native_context(&mut engine, crew, target);
        let mut expected = expected_linked_rows("Action");
        expected.extend(expected_linked_rows("Effect"));
        expected.extend([
            "Global late last".to_owned(),
            "Global late shared".to_owned(),
            "Global late first".to_owned(),
            "Global early last".to_owned(),
            "Global early first".to_owned(),
        ]);
        expected.extend(expected_linked_rows("Attach"));
        expected.extend(expected_linked_rows("Context"));
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.caption.clone())
                .collect::<Vec<_>>(),
            expected
        );
        for class in ["Action", "Effect", "Attach", "Context"] {
            assert!(
                menu.items
                    .iter()
                    .any(|item| item.caption == format!("{class} append shared")),
                "the appended same-name function wins for {class}"
            );
            assert!(
                menu.items.iter().all(|item| {
                    item.caption != format!("{class} local shared")
                        && item.caption != format!("{class} include shared")
                }),
                "overloaded {class} rows stay hidden"
            );
        }
        assert!(
            menu.items
                .iter()
                .all(|item| item.caption != "Global early shared"),
            "the later engine-global same-name function wins"
        );
    }

    #[test]
    fn native_context_classes_keep_cpp_order_conditions_and_commands() {
        let mut engine = clonk_engine("#strict 2\n");
        register_menu_image_definitions(&mut engine, &["ACIM", "FXIM", "ATIM", "ACTI", "CTXI"]);
        engine.register_test_definition(
                test_definition("AHST", "Action host", r#"
                #strict 2
                func ActionContextRide(menu, image, target) {
                    [Action|Image=ACIM:1|Condition=AllowAction]
                    return 1;
                }
                func AllowAction(menu, image, target) {
                    return GetID(this()) == AHST && GetID(menu) == CLNK && image == ACIM && GetID(target) == TARG;
                }
                "#),
            );
        engine.register_test_definition(
                test_definition("EHST", "Effect host", r#"
                #strict 2
                func FxGlowContextInspect(target, number, menu, image) {
                    [Effect|Image=FXIM:2|Condition=AllowEffect]
                    return 1;
                }
                func AllowEffect(target, number, menu, image) {
                    return GetID(this()) == EHST && GetID(target) == TARG && number == 7 && GetID(menu) == CLNK && image == FXIM;
                }
                "#),
            );
        let mut attached_definition = test_definition(
            "ATCH",
            "Attachment",
            r#"
        #strict 2
        func AttachContextDetach(menu, image, target) {
            [Attach|Image=ATIM:3|Condition=AllowAttach]
            return 1;
        }
        func AllowAttach(menu, image, target) {
            return GetID(this()) == ATCH && GetID(menu) == CLNK && image == ATIM && GetID(target) == TARG;
        }
        "#,
        );
        attached_definition.configure_actions(
            None,
            HashMap::from([(
                "Attached".to_string(),
                ActionSpec::default().with_procedure("attach"),
            )]),
        );
        engine.register_test_definition(attached_definition);
        let mut target_definition = test_definition(
            "TARG",
            "Target",
            r#"
        #strict 2
        func Activate(menu) {
            [Activate|Image=ACTI:4|Condition=AllowActivate]
            return 1;
        }
        func AllowActivate(menu, image) { return GetID(menu) == CLNK && image == ACTI; }
        func ContextInspect(menu) {
            [Context|Image=CTXI:5|Condition=AllowContext]
            return 1;
        }
        func AllowContext(menu, image) { return GetID(menu) == CLNK && image == CTXI; }
        "#,
        );
        target_definition.configure_actions(
            None,
            HashMap::from([("Use".to_string(), ActionSpec::default())]),
        );
        engine.register_test_definition(target_definition);
        let crew = register_player_crew(&mut engine);
        let action_host = engine.spawn_test_object(SpawnConfig::new("AHST"));
        let effect_host = engine.spawn_test_object(SpawnConfig::new("EHST"));
        let mut target_action = ActionState::new("Use");
        target_action.target = Some(action_host);
        let target = engine.spawn_test_object(
            SpawnConfig::new("TARG")
                .with_container(crew)
                .with_action(target_action),
        );
        let mut glow = crate::EffectState::new("Glow");
        glow.number = 7;
        glow.command_target = Some(effect_host.as_u64() as i32);
        test_object_mut(&mut engine, target)
            .state
            .effects
            .push(glow);
        let mut attached_action = ActionState::new("Attached");
        attached_action.target = Some(target);
        let attached =
            engine.spawn_test_object(SpawnConfig::new("ATCH").with_action(attached_action));

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(
            menu_captions(&menu),
            ["Action", "Effect", "Attach", "Activate", "Context"]
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.command.clone())
                .collect::<Vec<_>>(),
            vec![
                format!(
                    "ProtectedCall(Object({}),\"ActionContextRide\",this,Object({}))",
                    action_host.as_u64(),
                    target.as_u64()
                ),
                format!(
                    "ProtectedCall(Object({}),\"FxGlowContextInspect\",Object({}),7,Object({}),FXIM)",
                    effect_host.as_u64(),
                    target.as_u64(),
                    crew.as_u64()
                ),
                format!(
                    "ProtectedCall(Object({}),\"AttachContextDetach\",this,Object({}))",
                    attached.as_u64(),
                    target.as_u64()
                ),
                format!(
                    "ProtectedCall(Object({}),\"Activate\",this)",
                    target.as_u64()
                ),
                format!(
                    "ProtectedCall(Object({}),\"ContextInspect\",this)",
                    target.as_u64()
                ),
            ]
        );
        assert!(menu.items.iter().all(|item| item.item_id == "NONE"));
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.presentation_definition_id.as_deref())
                .collect::<Vec<_>>(),
            [
                Some("ACIM"),
                Some("FXIM"),
                Some("ATIM"),
                Some("ACTI"),
                Some("CTXI")
            ]
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.image.clone())
                .collect::<Vec<_>>(),
            (1..=5)
                .map(|index| crate::ObjectMenuImage::Indexed { index })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn native_context_activate_falls_back_and_equal_context_suppresses_it() {
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "ACTV",
            "Relic",
            "#strict 2\nfunc Activate(menu) { return 1; }\n",
        ));
        engine.register_test_definition(test_definition(
            "DUPL",
            "Duplicate",
            r#"
                #strict 2
                func Activate(menu) { [Use] return 1; }
                func ContextUse(menu) { [Use] return 1; }
                "#,
        ));
        let crew = register_player_crew(&mut engine);
        let relic = engine.spawn_test_object(SpawnConfig::new("ACTV").with_container(crew));

        let menu = open_native_context(&mut engine, crew, relic);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Relic");
        assert_eq!(
            menu.items[0].command,
            format!(
                "ProtectedCall(Object({}),\"Activate\",this)",
                relic.as_u64()
            )
        );
        assert_eq!(
            menu.items[0].image,
            crate::ObjectMenuImage::Object { object: relic }
        );
        assert_eq!(
            menu.items[0]
                .picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("ACTV")
        );
        assert_eq!(menu.items[0].picture_object, None);
        assert_eq!(menu.items[0].item_id, "NONE");

        let duplicate = engine.spawn_test_object(SpawnConfig::new("DUPL").with_container(crew));
        let menu = open_native_context(&mut engine, crew, duplicate);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Use");
        assert_eq!(
            menu.items[0].command,
            format!(
                "ProtectedCall(Object({}),\"ContextUse\",this)",
                duplicate.as_u64()
            )
        );
    }

    #[test]
    fn native_context_pushed_target_exposes_control_dig_double() {
        let mut engine = clonk_engine("#strict 2\n");
        engine.register_test_definition(test_definition(
            "MACH",
            "Machine",
            "#strict 2\nfunc ControlDigDouble(menu) { [Drill] return 1; }\n",
        ));
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("MACH"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action = ActionState::new("Push");
        engine.objects[crew_index].state.action.target = Some(target);

        let menu = open_native_context(&mut engine, crew, target);
        let drill = menu
            .items
            .iter()
            .find(|item| item.caption == "Drill")
            .test_value();
        assert_eq!(
            drill.command,
            format!(
                "ProtectedCall(Object({}),\"ControlDigDouble\",this)",
                target.as_u64()
            )
        );
    }

    #[test]
    fn context_container_rows_include_put_for_pushed_grab_put_target() {
        // C4MN_Context exposes Put while the Clonk is pushing the target when
        // the target definition has C4D_Grab_Put. The >1 inventory condition
        // supplies the same Put-all secondary command as containment
        // (C4ObjectMenu.cpp:335-359).
        let mut engine = clonk_engine("#strict\n");
        let mut container = test_definition("CONT", "Container", "#strict\n");
        container.set_category(crate::CATEGORY_VEHICLE);
        container.set_grab_put_get(crate::GRAB_PUT_GET_PUT);
        engine.register_test_definition(container);
        engine.register_test_script_definition("ITEM", "Item", "#strict\n");
        let crew = register_player_crew(&mut engine);
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(crew));
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(crew));
        let target = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action = ActionState::new("Push");
        engine.objects[crew_index].state.action.target = Some(target);

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu_captions(&menu), ["Put"]);
        assert_eq!(
            menu.items[0].command,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 0, 0) && ExecuteCommand()",
                target.as_u64()
            )
        );
        assert_eq!(
            menu.items[0].command2,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 1000, 0) && ExecuteCommand()",
                target.as_u64()
            )
        );
        assert_eq!(
            menu.items[0]
                .picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("ITEM"),
            "Put composes the first carried object's refill-time picture",
        );
    }

    #[test]
    fn context_container_rows_include_contents_for_pushed_grab_get_target() {
        // C4ObjectMenu::RefillInternal draws the target into the Contents
        // row before Add takes ownership of the facet
        // (C4ObjectMenu.cpp:361-373).
        let mut engine = clonk_engine("#strict\n");
        let mut container = test_definition("CONT", "Container", "#strict\n");
        container.set_category(crate::CATEGORY_VEHICLE);
        container.set_grab_put_get(crate::GRAB_PUT_GET_GET);
        engine.register_test_definition(container);
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action = ActionState::new("Push");
        engine.objects[crew_index].state.action.target = Some(target);

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu_captions(&menu), ["Contents"]);
        assert_eq!(
            menu.items[0].command,
            format!(
                "SetCommand(this,\"Get\",Object({}),0,0,,2)&&ExecuteCommand()",
                target.as_u64()
            )
        );
        assert_eq!(
            menu.items[0]
                .picture_snapshot
                .as_ref()
                .map(|picture| picture.definition_id.as_str()),
            Some("CONT"),
            "Contents keeps the target picture captured during refill",
        );
    }

    #[test]
    fn context_container_rows_include_contents_for_friendly_remote_target() {
        let mut engine = clonk_engine("#strict\n");
        let mut container = structure_definition("CONT", "Container", "#strict\n");
        container.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(container);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.register_test_player(PlayerConfig::new(2, "Friend"));
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let target = engine.spawn_test_object(SpawnConfig::new("CONT").with_owner(2));

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu_captions(&menu), ["Contents"]);

        engine.set_hostility(1, 2, true).test_value();
        let menu = open_native_context(&mut engine, crew, target);
        assert!(
            menu.items.iter().all(|item| item.caption != "Contents"),
            "hostile ownership must suppress the remote Contents row"
        );
    }

    #[test]
    fn context_container_rows_preserve_contained_put_contents_and_exit() {
        let mut engine = clonk_engine("#strict\n");
        let mut container = structure_definition("CONT", "Container", "#strict\n");
        container.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(container);
        engine.register_test_script_definition("ITEM", "Item", "#strict\n");
        let crew = register_player_crew(&mut engine);
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(crew));
        let target = engine.spawn_test_object(SpawnConfig::new("CONT"));
        contain_object(&mut engine, crew, target);

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(menu_captions(&menu), ["Put", "Contents", "Exit"]);
    }

    #[test]
    fn native_context_effect_commands_use_live_numbers_and_callback_hosts() {
        let mut engine = clonk_engine("#strict 2\n");
        register_menu_image_definitions(&mut engine, &["LIMG", "DIMG", "GIMG"]);
        engine.register_test_definition(
                test_definition("LHST", "Live host", r#"
                #strict 2
                func FxLiveContextOpen(target, number, menu, image) {
                    [Live|Image=LIMG|Condition=AllowLive]
                    return 1;
                }
                func AllowLive(target, number, menu, image) {
                    return GetID(target) == TARG && number == 4 && GetID(menu) == CLNK && image == LIMG;
                }
                "#),
            );
        engine.register_test_definition(
                test_definition("DHST", "Definition host", "#strict 2\nfunc FxDefContextOpen(target, number, menu, image) { [Definition|Image=DIMG] return 1; }\n"),
            );
        engine.register_test_script_definition("TARG", "Target", "#strict 2\n");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Context.c".to_string(),
                r#"#strict 2
global func FxWorldContextOpen(target, number, menu, image) {
    [Global|Image=GIMG|Condition=AllowWorld]
    return 1;
}
func AllowWorld(target, number, menu, image) {
    return GetID(target) == TARG && number == 11 && GetID(menu) == CLNK && image == GIMG && GetMenu(menu) == 14;
}
"#
                .to_string(),
            )]),
            1
        );
        let crew = register_player_crew(&mut engine);
        let live_host = engine.spawn_test_object(SpawnConfig::new("LHST"));
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let mut live = crate::EffectState::new("Live");
        live.number = 4;
        live.command_target = Some(live_host.as_u64() as i32);
        let mut definition = crate::EffectState::new("Def");
        definition.number = 8;
        definition.command_id = Some("DHST".to_string());
        let mut global = crate::EffectState::new("World");
        global.number = 11;
        test_object_mut(&mut engine, target).state.effects = vec![live, definition, global];

        let menu = open_native_context(&mut engine, crew, target);
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.command.clone())
                .collect::<Vec<_>>(),
            vec![
                format!(
                    "ProtectedCall(Object({}),\"FxLiveContextOpen\",Object({}),4,Object({}),LIMG)",
                    live_host.as_u64(),
                    target.as_u64(),
                    crew.as_u64()
                ),
                format!(
                    "DefinitionCall(DHST, \"FxDefContextOpen\", Object({}),8,Object({}),DIMG)",
                    target.as_u64(),
                    crew.as_u64()
                ),
                format!(
                    "global->~FxWorldContextOpen(Object({}),11,Object({}),GIMG)",
                    target.as_u64(),
                    crew.as_u64()
                ),
            ]
        );
    }

    #[test]
    fn native_context_build_info_follows_rule_rotation_and_row_order() {
        let mut engine = clonk_engine("#strict 2\n");
        let mut site = test_definition(
            "SITE",
            "Site",
            "#strict 2\nfunc ContextInspect(menu) { [Inspect] return 1; }\n",
        );
        site.set_constructable(true);
        site.set_description(Some("An unfinished site.".to_string()));
        engine.register_test_definition(site);
        let crew = register_player_crew(&mut engine);
        let site = engine
            .spawn_test_object(SpawnConfig::new("SITE").with_construction(crate::FULL_CON / 2));
        assert_ne!(test_object(&engine, site).state.ocf & ocf::CONSTRUCT, 0);
        engine.set_construction_needs_material(true);

        let menu = open_native_context(&mut engine, crew, site);
        assert_eq!(
            menu_captions(&menu),
            ["Inspect", "Construction material", "Info"]
        );
        let build = &menu.items[1];
        assert_eq!(
            build.command,
            format!(
                "PlayerMessage(GetOwner(), Object({})->GetNeededMatStr(), Object({}))",
                site.as_u64(),
                site.as_u64()
            )
        );
        assert_eq!(build.symbol, crate::ObjectMenuSymbol::Construction);

        test_object_mut(&mut engine, crew).state.rotation = 1;
        let menu = open_native_context(&mut engine, crew, site);
        assert!(
            menu.items
                .iter()
                .all(|item| item.caption != "Construction material"),
            "the C++ gate checks the menu object's rotation"
        );
    }

    #[test]
    fn native_context_clonk_submenu_threshold_counts_every_class() {
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            r#"
#strict 2
func ContextOne(menu) { [One] return 1; }
func ContextTwo(menu) { [Two] return 1; }
func FxPulseContextThree(target, number, menu, image) { [Pulse] return 1; }
"#,
        );
        let mut base = structure_definition("BASE", "Base", "#strict 2\n");
        base.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(base);
        let (crew, base) = contain_player_crew(&mut engine, "BASE");
        let mut pulse = crate::EffectState::new("Pulse");
        pulse.number = 3;
        pulse.command_target = Some(crew.as_u64() as i32);
        test_object_mut(&mut engine, crew).state.effects.push(pulse);

        let menu = open_native_context(&mut engine, crew, base);
        assert_eq!(menu_captions(&menu), ["Contents", "CLNK", "Exit"]);
        let submenu = &menu.items[1];
        assert_eq!(
            submenu.command,
            "SetCommand(this,\"Context\",,0,0,this)&&ExecuteCommand()"
        );
        assert_eq!(submenu.item_id, "NONE");
        assert_eq!(submenu.presentation_definition_id.as_deref(), Some("CLNK"));
    }

    #[test]
    fn clonk_context_construction_opens_the_native_menu() {
        // Reduced shipped CLNK::ContextConstruction: its definition-less
        // SetCommand followed by synchronous ExecuteCommand opens
        // C4MN_Construction and finishes the command successfully
        // (Objects.c4d/Crew.c4d/Clonk.c4d/Script.c:628-634).
        let script = r#"
#strict 2
public func ContextConstruction(object caller)
{
    [Construction|Image=CXCN|Desc=Construct a building.]
    SetCommand(this(), "Construct");
    ExecuteCommand();
    return 1;
}
"#;
        let mut engine = Engine::new();
        let mut definition = test_definition("CLNK", "Clonk", script);
        definition.configure_actions(Some("Walk".to_string()), clonk_actions());
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(PhysicalInfo {
            can_construct: 1,
            ..Default::default()
        });
        engine.register_test_definition(definition);
        engine.register_test_player(PlayerConfig::new(1, "Builder"));
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        assert!(engine
            .player_context_command(1, crew)
            .expect("queue context command"));
        engine.execute_object_command_now(crew).test_value();
        let construction_index = test_menu(&engine, crew)
            .items
            .iter()
            .position(|item| item.command.contains("ContextConstruction"))
            .test_value() as i32;
        for _ in 0..construction_index {
            engine.player_in_com(1, COM_RIGHT, 0).test_value();
        }

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(1));
        assert_eq!(menu.caption, "Player Builder|has no construction plans.");
        assert_eq!(menu.extra, crate::ObjectMenuExtra::Components);
        assert_eq!(menu.selection, -1);
        assert!(menu.items.is_empty());
        assert!(test_snapshot(&engine, crew)
            .command_stack
            .command_names()
            .is_empty());
        assert!(
            engine.pending_menu_requests.is_empty(),
            "native Construction requests are consumed inside clonk-engine"
        );
    }

    #[test]
    fn native_construction_menu_filters_knowledge_and_exposes_drag_rows() {
        let mut engine = Engine::new();
        register_builder_clonk(&mut engine, "CLNK", "#strict 2\n");
        for id in ["CXCN", "WOOD", "METL"] {
            engine.register_test_script_definition(id, id, "#strict 2\n");
        }
        let mut elevator = structure_definition(
            "ELEV",
            "Elevator",
            r#"
        #strict 2
        public func GetCustomComponents(object builder)
        {
            return [WOOD, WOOD, WOOD, WOOD, METL, METL];
        }
        "#,
        );
        elevator.set_constructable(true);
        elevator.set_description(Some("Lift\npeople\rquickly.".to_string()));
        elevator.set_components(vec![crate::DefinitionComponent {
            id: "WOOD".to_string(),
            count: 99,
        }]);
        engine.register_test_definition(elevator);

        let facade = structure_definition("FACA", "Facade", "#strict 2\n");
        engine.register_test_definition(facade);

        let mut vehicle = test_definition("VEHI", "Vehicle", "#strict 2\n");
        vehicle.set_category(crate::CATEGORY_VEHICLE);
        vehicle.set_constructable(true);
        engine.register_test_definition(vehicle);

        engine.register_test_player(PlayerConfig::new(1, "Builder"));
        for definition_id in ["VEHI", "ELEV", "FACA"] {
            engine.grant_player_knowledge(1, definition_id).test_value();
        }
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.test_object_index(crew);
        engine.open_construction_menu(crew_index).test_value();

        let menu = engine.cursor_object_menu(1).test_value().1;
        assert_eq!(menu.identification, Value::Int(1));
        assert_eq!(menu.symbol_id, "CXCN");
        assert_eq!(menu.extra, crate::ObjectMenuExtra::Components);
        assert!(!menu.permanent);
        assert_eq!(menu.command_object, Some(crew));
        assert_eq!(menu.selection, 0);
        assert_eq!(
            menu_item_ids(menu),
            ["ELEV", "FACA"],
            "only known structures appear, in knowledge order"
        );
        let elevator = &menu.items[0];
        assert_eq!(elevator.caption, "Construction: Elevator");
        assert_eq!(elevator.info_caption, "Lift people|quickly.");
        assert_eq!(
            elevator.command,
            "SetCommand(this, \"Construct\",,0,0,,ELEV)"
        );
        assert_eq!(elevator.count, 12_345_678);
        assert_eq!(
            elevator.components,
            [
                crate::ObjectMenuComponent {
                    definition_id: "WOOD".to_string(),
                    count: 4,
                },
                crate::ObjectMenuComponent {
                    definition_id: "METL".to_string(),
                    count: 2,
                },
            ]
        );

        let drag = engine.object_menu_construction_drag(1, 0).test_value();
        assert_eq!(drag.menu_object_id, crew);
        assert_eq!(drag.definition_id, "ELEV");
        assert_eq!(
            drag.definition_c4id,
            clonk_script::c4_id_raw("ELEV") as u32 as i32
        );
        assert_eq!(
            engine.object_menu_construction_drag(1, 1),
            None,
            "known structures remain menu rows even when not constructable"
        );

        test_object_mut(&mut engine, crew)
            .state
            .menu
            .as_mut()
            .test_value()
            .items[1]
            .presentation_definition_id = Some("ELEV".to_string());
        assert_eq!(
            engine.object_menu_construction_drag(1, 1),
            None,
            "drag eligibility never uses the presentation fallback"
        );

        engine.player_in_com(1, COM_MENU_ENTER, 0).test_value();
        assert_eq!(
            engine.debug_object_menu(crew.as_u64()),
            Some(None),
            "the nonpermanent construction menu closes on entry"
        );
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "Construct");
        assert_eq!(
            commands[0].data,
            CommandData::Integer(clonk_script::c4_id_raw("ELEV") as u32 as i32)
        );
    }

    #[test]
    fn ordinary_construction_menu_event_is_consumed_by_the_crew_owner() {
        let mut engine = Engine::new();
        register_builder_clonk(&mut engine, "CLNK", "#strict 2\n");
        let mut tower = structure_definition("TOWR", "Tower", "#strict 2\n");
        tower.set_constructable(true);
        engine.register_test_definition(tower);
        engine.register_test_player(PlayerConfig::new(1, "Owner"));
        engine.register_test_player(PlayerConfig::new(2, "Spoofed"));
        engine.grant_player_knowledge(1, "TOWR").test_value();
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine
            .apply_command_event(crate::command::CommandEvent::OpenMenu(crate::MenuRequest {
                crew_id: crew,
                owner: 2,
                kind: crate::MenuRequestKind::Construction,
            }))
            .test_value();

        assert!(engine.pending_menu_requests.is_empty());
        let menu = engine.cursor_object_menu(1).test_value().1;
        assert_eq!(menu.caption, "Player Owner|has no construction plans.");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].item_id, "TOWR");
    }

    #[test]
    fn construction_site_preview_reuses_terrain_support_and_overlap_checks() {
        let mut engine = Engine::new();
        let mut tower = structure_definition("TOWR", "Tower", "#strict 2\n");
        tower.set_constructable(true);
        tower.set_shape_rect(Some(crate::DefinitionRect::new(-10, -40, 20, 40)));
        engine.register_test_definition(tower);
        let mut facade = structure_definition("FACA", "Facade", "#strict 2\n");
        facade.set_shape_rect(Some(crate::DefinitionRect::new(-5, -5, 10, 10)));
        engine.register_test_definition(facade);
        engine.set_landscape(Landscape::flat(100, 50));

        let site = Vector2::new(50, 50);
        assert!(engine.construction_site_valid("TOWR", site));
        assert!(!engine.construction_site_valid("NONE", site));
        assert!(!engine.construction_site_valid("FACA", site));
        assert!(
            !engine.construction_site_valid("TOWR", Vector2::new(5, 50)),
            "a construction rectangle crossing a closed side sees vehicle-solid border pixels"
        );
        let mut open_side = Landscape::flat(100, 50);
        open_side.set_border_open(60, 0, true, false);
        engine.set_landscape(open_side);
        assert!(
            engine.construction_site_valid("TOWR", Vector2::new(5, 50)),
            "the same rectangle may cross a side that is open throughout the sampled area"
        );
        assert!(
            !engine.construction_site_valid("TOWR", Vector2::new(50, 40)),
            "the site needs two rows of support within its five-pixel strip"
        );

        engine.spawn_test_object(
            SpawnConfig::new("FACA")
                .with_category(crate::CATEGORY_STRUCTURE)
                .with_position(Vector2::new(50, 30)),
        );
        assert!(
            !engine.construction_site_valid("TOWR", site),
            "an overlapping live structure vetoes the site"
        );
    }

    #[test]
    fn construction_site_visibility_matches_repellers_generators_and_target_fallback() {
        let mut engine = Engine::new();
        for (id, closed) in [("VIEW", 0), ("HUT1", 1)] {
            let mut definition = test_definition(id, id, "#strict 2\n");
            definition.set_closed_container(closed);
            engine.register_test_definition(definition);
        }
        engine.register_test_player(PlayerConfig::new(1, "Builder"));
        engine.register_test_player(PlayerConfig::new(2, "Target owner"));
        let mut landscape = Landscape::flat(200, 100);
        landscape.set_world_height(200);
        engine.set_landscape(landscape);

        assert!(engine.construction_site_visible(1, Vector2::new(199, 199)));
        assert!(!engine.construction_site_visible(1, Vector2::new(200, 199)));
        assert!(!engine.construction_site_visible(1, Vector2::new(199, 200)));
        assert!(!engine.construction_site_visible(99, Vector2::new(50, 50)));

        let repeller = engine.spawn_test_object(
            SpawnConfig::new("VIEW")
                .with_owner(1)
                .with_position(Vector2::new(50, 50))
                .with_plr_view_range(30),
        );
        // The repeller above already entered the list through the ordinary
        // spawn path, so this enabling edge's rebuild would be a no-op.
        let _ = engine.player_mut(1).test_value().set_fog_of_war(true);
        assert!(engine.construction_site_visible(1, Vector2::new(79, 50)));
        assert!(
            !engine.construction_site_visible(1, Vector2::new(80, 50)),
            "FoWIsVisible uses strict distance less-than"
        );

        let generator = engine.spawn_test_object(
            SpawnConfig::new("VIEW")
                .with_owner(1)
                .with_position(Vector2::new(55, 50))
                .with_plr_view_range(-10)
                .with_color_modulation(0xff00_0000),
        );
        assert!(
            engine.construction_site_visible(1, Vector2::new(50, 50)),
            "a faded generator paints darkness but does not block visibility"
        );
        let generator_index = engine.test_object_index(generator);
        engine.objects[generator_index].state.color_modulation = 0;
        assert!(
            !engine.construction_site_visible(1, Vector2::new(50, 50)),
            "an opaque generator overrides a covering repeller"
        );
        engine.objects[generator_index].state.color_modulation = 0xff00_0000;

        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));
        contain_object(&mut engine, repeller, hut);
        assert!(
            !engine.construction_site_visible(1, Vector2::new(50, 50)),
            "ClosedContainer=1 suppresses a contained FoW source"
        );

        let target = engine.spawn_test_object(
            SpawnConfig::new("VIEW")
                .with_owner(2)
                .with_position(Vector2::new(100, 100)),
        );
        {
            let player = engine.player_mut(1).test_value();
            player.set_cursor(Some(repeller));
            player.set_view_target(Some(target));
        }
        assert!(
            engine.construction_site_visible(1, Vector2::new(129, 100)),
            "a zero-range target falls back to the cursor's range"
        );
        assert!(!engine.construction_site_visible(1, Vector2::new(130, 100)));
        contain_object(&mut engine, target, hut);
        assert!(
            !engine.construction_site_visible(1, Vector2::new(100, 100)),
            "the target-view source obeys ClosedContainer too"
        );
    }

    #[test]
    fn construction_drag_image_selects_picture_or_raw_main_face() {
        let sprite_pixels = (0_u8..16)
            .flat_map(|value| [value, 0, 0, 255])
            .collect::<Vec<_>>();
        let mut main = test_definition("MAIN", "Main face", "#strict 2\n");
        main.set_shape_rect(Some(crate::DefinitionRect::new(-3, -4, 2, 1)));
        main.set_graphics_scale(2.0);
        main.set_sprite_image(Some(crate::DefinitionSpriteImage {
            width: 4,
            height: 4,
            pixels: std::sync::Arc::from(sprite_pixels.into_boxed_slice()),
            color_mask: None,
        }));

        let picture_sprite_pixels = (32_u8..48)
            .flat_map(|value| [value, 0, 0, 255])
            .collect::<Vec<_>>();
        let menu_picture_pixels = std::sync::Arc::from(vec![9, 8, 7, 255].into_boxed_slice());
        let mut picture = test_definition("PICT", "Picture", "#strict 2\n");
        picture.drag_image_picture = 1;
        picture.set_graphics_scale(2.0);
        picture.set_picture(Some(crate::DefinitionPicture {
            x: 1,
            y: 2,
            width: 2,
            height: 1,
        }));
        picture.set_sprite_image(Some(crate::DefinitionSpriteImage {
            width: 4,
            height: 4,
            pixels: std::sync::Arc::from(picture_sprite_pixels.into_boxed_slice()),
            color_mask: None,
        }));
        picture.set_picture_image(Some(crate::DefinitionPictureImage {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::clone(&menu_picture_pixels),
            color_mask: None,
        }));

        let mut engine = Engine::new();
        engine.register_test_definition(main);
        engine.register_test_definition(picture);

        let main = engine
            .definition_construction_drag_image("MAIN")
            .test_value();
        assert_eq!((main.width(), main.height()), (2, 1));
        assert_eq!(
            main.pixels().len(),
            2 * 4,
            "Shape x/y and GraphicsScale are ignored; raw width/height crop from Graphics origin"
        );
        let picture = engine
            .definition_construction_drag_image("PICT")
            .test_value();
        assert_eq!((picture.width(), picture.height()), (2, 1));
        assert_eq!(
            picture.pixels().as_ref(),
            &[41, 0, 0, 255, 42, 0, 0, 255],
            "DragImagePicture crops the raw PictureRect from Graphics, not the scaled menu image"
        );
        assert_ne!(picture.pixels().as_ref(), menu_picture_pixels.as_ref());
        assert!(engine.definition_construction_drag_image("NONE").is_none());
    }

    #[test]
    fn mouse_context_command_keeps_viewport_location_and_zero_axis_sentinel() {
        // C4MouseControl passes the clicked object as Target2 with Add mode;
        // self-targeting must not exclude the cursor as ordinary Target does.
        // C4Command::Context then installs non-permanent C4MN_Context and
        // applies Free/SetLocation only when both coordinates are nonzero
        // (C4MouseControl.cpp:1253-1260; C4Command.cpp:1076-1090).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "MCLK",
            r#"
#strict 2
public func ContextMagic(object caller)
{
    [Magic|Image=MCMS|Desc=Open the spell menu.]
    return 1;
}
"#,
        );
        engine.register_test_player(PlayerConfig::new(1, "Mage"));
        let mage = spawn_crew(&mut engine, "MCLK", 1);

        engine
            .execute_player_command(
                1,
                CommandId::Context as i32,
                17,
                23,
                0,
                mage.as_u64() as i32,
                0,
                C4P_COMMAND_ADD,
            )
            .test_value();
        assert_eq!(
            test_snapshot(&engine, mage).command_stack.command_names(),
            ["Context"]
        );
        engine.execute_object_command_now(mage).test_value();
        assert!(
            engine.pending_menu_requests.is_empty(),
            "context requests must be consumed into the native menu: {:?}",
            engine.pending_menu_requests
        );

        let menu = test_menu(&engine, mage);
        assert_eq!(menu.identification, Value::Int(14));
        assert_eq!(menu.style, 1);
        assert!(!menu.permanent);
        assert_eq!(menu.location, Some(Vector2::new(17, 23)));
        let mut prior_runtime_id = menu.runtime_id;
        assert_ne!(prior_runtime_id, 0);
        let restored: crate::ObjectMenuState = serde_json::from_value(
            serde_json::to_value(&menu).expect("serialize synchronized menu state"),
        )
        .test_value();
        assert_eq!(restored.location, menu.location);
        assert_ne!(restored.runtime_id, menu.runtime_id);
        assert_eq!(restored, menu, "runtime identities are not semantic state");
        assert_eq!(menu.command_object, Some(mage));
        assert!(menu.items.iter().any(|item| {
            item.caption == "Magic"
                && item.command.contains("ContextMagic")
                && item.command.contains(&mage.as_u64().to_string())
        }));

        for (x, y) in [(0, 23), (17, 0)] {
            engine.close_object_menu(mage, true).test_value();
            engine
                .execute_player_command(
                    1,
                    CommandId::Context as i32,
                    x,
                    y,
                    0,
                    mage.as_u64() as i32,
                    0,
                    C4P_COMMAND_ADD,
                )
                .test_value();
            engine.execute_object_command_now(mage).test_value();
            let menu = test_menu(&engine, mage);
            assert_eq!(
                menu.location, None,
                "x={x}, y={y} keeps default Right|Bottom alignment"
            );
            assert_ne!(
                menu.runtime_id, prior_runtime_id,
                "each ActivateMenu allocation gets a distinct presentation identity"
            );
            prior_runtime_id = menu.runtime_id;
        }
    }

    /// A script that opens a one-column Context menu and counts the steps it
    /// is offered, reporting `handled` for each.
    fn step_menu_script(handled: &str) -> String {
        format!(
            r#"#strict
            local steps;
            func OnMenuStep(int iDelta, object pMenuObject) {{
              steps = steps + iDelta;
              return {handled};
            }}
            func Open(int iStyle) {{
              CreateMenu(CLNK, this(), this(), 0, "Choose", 0, iStyle);
              AddMenuItem("A", "Nop()", CLNK, this());
              AddMenuItem("B", "Nop()", CLNK, this());
              AddMenuItem("C", "Nop()", CLNK, this());
              return SelectMenuItem(1, this());
            }}
            func Nop() {{ return 1; }}
            "#
        )
    }

    fn open_step_menu(engine: &mut Engine, style: i32) -> ObjectId {
        let crew = spawn_crew(engine, "CLNK", 1);
        let index = engine.test_object_index(crew);
        engine.call_test_object_function(index, "Open", vec![Value::Int(style)]);
        crew
    }

    fn step_count(engine: &Engine, crew: ObjectId) -> Value {
        let index = engine.test_object_index(crew);
        engine.objects[index]
            .state
            .local_vars
            .get("steps")
            .cloned()
            .unwrap_or(Value::Nil)
    }

    #[test]
    fn a_one_column_script_menu_offers_left_and_right_as_a_step_before_moving() {
        // C4Menu::Control gives Left/Right exactly the deltas Up/Down already
        // have once Columns == 1 (C4Menu.cpp:433-457), so they carry no
        // distinct meaning in a Context menu. The port offers them to the
        // menu's own command object as ~OnMenuStep first, modelled on
        // C4ObjectMenu::OnSelectionChanged (C4ObjectMenu.cpp:93-104); a
        // truthy return means the script consumed the input and the
        // selection stays put.
        let mut engine = clonk_engine(&step_menu_script("true"));
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = open_step_menu(&mut engine, 1);

        engine.player_in_com(1, COM_MENU_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_MENU_LEFT, 0).test_value();
        engine.player_in_com(1, COM_MENU_LEFT, 0).test_value();

        assert_eq!(
            step_count(&engine, crew),
            Value::Int(-1),
            "right offers +1 and left -1"
        );
        assert_eq!(
            test_menu(&engine, crew).selection,
            1,
            "a handled step must not also move the selection"
        );
    }

    #[test]
    fn an_unclaimed_step_still_moves_the_selection_exactly_as_it_did() {
        // A falsy return means the script looked at the com and did not want
        // it, so COM_MenuLeft/Right must behave as C4Menu::Control always did
        // — including the wrap at either end (C4Menu.cpp:444-457).
        let mut engine = clonk_engine(&step_menu_script("false"));
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = open_step_menu(&mut engine, 1);
        let selection = |engine: &Engine| test_menu(engine, crew).selection;

        for (com, expected) in [
            (COM_MENU_RIGHT, 2),
            (COM_MENU_RIGHT, 0), // wraps off the end
            (COM_MENU_LEFT, 2),  // wraps off the front
            (COM_MENU_LEFT, 1),
        ] {
            engine.player_in_com(1, com, 0).test_value();
            assert_eq!(selection(&engine), expected);
        }
        assert_eq!(
            step_count(&engine, crew),
            Value::Int(0),
            "the script was still offered every step: +1 +1 -1 -1"
        );
    }

    #[test]
    fn a_multi_column_menu_never_offers_a_step() {
        // Left/Right are real horizontal navigation once Columns > 1, which
        // is C4MN_Style_Normal's five-wide grid (C4Menu.cpp:359-365). The
        // callback must not take them away from it.
        let mut engine = clonk_engine(&step_menu_script("true"));
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = open_step_menu(&mut engine, 0);

        engine.player_in_com(1, COM_MENU_RIGHT, 0).test_value();

        assert_eq!(
            step_count(&engine, crew),
            Value::Nil,
            "OnMenuStep is never reached in a grid"
        );
        assert_eq!(
            test_menu(&engine, crew).selection,
            2,
            "the grid keeps its own horizontal move"
        );
    }

    #[test]
    fn auto_context_put_row_deposits_the_first_carried_object() {
        // C4MN_Context starts with Put when the command object is carrying
        // something inside a container (C4ObjectMenu.cpp:335-359). Because
        // it is the first selected row, Throw enters it and immediately
        // executes the Put command on the contained Clonk.
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT2", "Hut", "#strict\n");
        engine.register_test_script_definition("FLAG", "Flag", "#strict\n");
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
        let flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        contain_object(&mut engine, crew, hut);

        engine.execute_player_controls().test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 0);
        assert_eq!(
            menu.items.first().map(|item| item.caption.as_str()),
            Some("Put")
        );
        assert_eq!(menu.items[0].symbol, crate::ObjectMenuSymbol::Put);
        assert_eq!(
            menu.items[0].command,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 0, 0) && ExecuteCommand()",
                hut.as_u64()
            )
        );

        engine.player_in_com(1, COM_THROW, 0).test_value();

        assert_eq!(
            test_snapshot(&engine, flag).container,
            Some(hut),
            "the selected Put row deposits the carried flag"
        );

        let second_flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        let third_flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(crew));
        engine.tick_without_snapshot().test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu.items[0].command2,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 1000, 0) && ExecuteCommand()",
                hut.as_u64()
            )
        );

        engine.player_in_com(1, COM_SPECIAL2, 0).test_value();
        let command = test_snapshot(&engine, crew)
            .command_stack
            .command_views()
            .into_iter()
            .next()
            .test_value();
        assert_eq!(command.name, "Put");
        assert_eq!(command.tx, Some(2));
        let resolved_item = command.target2.test_value();
        assert!(
            [second_flag, third_flag].contains(&resolved_item),
            "Put-all resolves one of the carried flags"
        );
        engine.tick_without_snapshot().test_value();
        engine.tick_without_snapshot().test_value();
        engine.tick_without_snapshot().test_value();

        for flag in [second_flag, third_flag] {
            assert_eq!(
                test_snapshot(&engine, flag).container,
                Some(hut),
                "Put-all deposits every carried object"
            );
        }
        assert!(
            test_snapshot(&engine, crew).command_stack.is_empty(),
            "Put-all finishes after observing the final item in the target"
        );
    }

    #[test]
    fn selecting_auto_context_exit_row_exits_the_building() {
        // C4MN_Context's Exit row runs PlayerObjectCommand("Exit") and
        // ExecuteCommand on the menu object (C4ObjectMenu.cpp:426-433).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        // The context Exit row is immediate only while the door is open;
        // otherwise C++ first asks ActivateEntrance (C4Command.cpp:624-665).
        engine.objects[hut_index].state.entrance_status = true;
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        for _ in 0..3 {
            engine.player_in_com(1, COM_RIGHT, 0).test_value();
        }
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.items[menu.selection as usize].caption, "Exit");

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let crew_after_evaluation = test_snapshot(&engine, crew);
        assert_eq!(
            crew_after_evaluation.container,
            Some(hut),
            "the row's synchronous ExecuteCommand consumes Exit's InitEvaluation"
        );
        assert_eq!(
            crew_after_evaluation.command_stack.command_names(),
            ["Exit"]
        );
        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));

        engine.tick_without_snapshot().test_value();
        let crew_after_exit = test_snapshot(&engine, crew);
        assert_eq!(
            crew_after_exit.container, None,
            "the selected Exit row exits on the following object execution"
        );
        assert!(crew_after_exit.command_stack.is_empty());
    }

    #[test]
    fn contained_context_buy_entry_opens_the_buy_menu() {
        // The C4MN_Context Buy row runs a data-less C4CMD_Buy, which opens
        // C4MN_Buy on its Target before succeeding (C4ObjectMenu.cpp:
        // 376-387; C4Command.cpp:1987-2004). Menu controls are converted
        // ahead of gameplay by C4Player::InCom (C4Player.cpp:1502-1513).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
        lorry.set_value(25);
        engine.register_test_definition(lorry);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .test_value();
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        test_object_mut(&mut engine, hut).state.base = 1;
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(4), "C4MN_Buy");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].item_id, "LORY");
    }

    #[test]
    fn contained_context_info_entry_opens_the_info_menu() {
        // The Context Info row executes ShowInfo(target), which calls
        // ActivateMenu(C4MN_Info) on the command object and adds the
        // target's info string (C4ObjectMenu.cpp:410-423;
        // C4Script.cpp:3332-3336; C4Object.cpp:2008-2027).
        let mut engine = clonk_engine("#strict\n");
        let mut hut = structure_definition("HUT3", "Hut", "#strict\n");
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        hut.set_description(Some("A sturdy wooden hut.".to_string()));
        engine.register_test_definition(hut);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        test_object_mut(&mut engine, hut).state.base = 1;
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();
        for _ in 0..3 {
            engine.player_in_com(1, COM_RIGHT, 0).test_value();
        }

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(15), "C4MN_Info");
        assert_eq!(menu.style, 2, "C4MN_Style_Info");
        assert!(menu.permanent);
        assert_eq!(menu.title_symbol, crate::ObjectMenuSymbol::InfoTitle);
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Hut");
        assert_eq!(menu.items[0].info_caption, "A sturdy wooden hut.");
        assert!(menu.items[0].selectable);
        assert_eq!(menu.items[0].picture_object, Some(hut));
    }

    /// `GetInfoString` walks the **live** effect list, so an `Fx*Info` hook that
    /// adds an effect behind the cursor is visited in the same pass
    /// (`src/C4Object.cpp:6140-6158`, oracle `7d43b47`):
    ///
    /// ```cpp
    /// for (C4Effect *pEff = pEffects; pEff; pEff = pEff->pNext)
    /// {
    ///     C4Value vInfo = pEff->DoCall(this, PSFS_FxInfo);
    /// ```
    ///
    /// `pEff->pNext` is read *after* `DoCall` returned, so the successor is
    /// whatever the callback left behind. Iterating a snapshot taken before the
    /// first callback shows the list as it was and silently drops the addition —
    /// which is what the port did until clonk-org/clonk-rs#562.
    ///
    /// Note there is deliberately no dead-effect filter: `C4Effect::DoCall`
    /// (C4Effect.cpp:439-457) has no `IsDead` gate and `GetInfoString` adds
    /// none, unlike `DoDamage`.
    #[test]
    fn object_info_menu_walks_effects_added_during_an_info_callback() {
        let mut engine = clonk_engine("#strict\n");
        let mut target = test_definition(
            "TARG",
            "Target",
            r#"#strict
        func FxGlowInfo(object target, int number)
        {
            // Appended after the cursor, so the live walk must reach it.
            AddEffect("Late", target, 30, 0, target);
            return "Glowing.";
        }
        func FxLateInfo(object target, int number) { return "Added mid-walk."; }
        "#,
        );
        target.set_description(Some("Base description.".to_string()));
        engine.register_test_definition(target);
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let target_index = engine.test_object_index(target);
        let mut glow = crate::EffectState::new("Glow");
        glow.number = 7;
        // Lower than the addition below: C4Effect inserts at the head unless
        // Abs(head->iPriority) < iPrio, so only a higher-priority addition
        // lands *after* the cursor where the live walk can still reach it.
        glow.priority = 10;
        glow.command_target = Some(target.as_u64() as i32);
        engine.objects[target_index].state.effects = vec![glow];

        let crew_index = engine.test_object_index(crew);
        engine
            .open_object_info_menu(crew_index, target_index)
            .test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu.items[0].info_caption, "Base description.|Glowing.|Added mid-walk.",
            "the effect added by FxGlowInfo must be visited by the same walk"
        );
    }

    #[test]
    fn object_info_menu_appends_script_and_native_effect_info_in_list_order() {
        let mut engine = clonk_engine("#strict\n");
        let mut target = test_definition(
            "TARG",
            "Target",
            "#strict\nfunc FxGlowInfo(object target, int number) { return \"Glowing.\"; }\n",
        );
        target.set_description(Some("Base description.".to_string()));
        engine.register_test_definition(target);
        let crew = register_player_crew(&mut engine);
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let target_index = engine.test_object_index(target);
        let mut glow = crate::EffectState::new("Glow");
        glow.number = 7;
        glow.command_target = Some(target.as_u64() as i32);
        let mut fire = crate::EffectState::new(crate::C4FX_FIRE);
        fire.number = 8;
        fire.command_target = Some(target.as_u64() as i32);
        engine.objects[target_index].state.effects = vec![glow, fire];

        let crew_index = engine.test_object_index(crew);
        engine
            .open_object_info_menu(crew_index, target_index)
            .test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu.items[0].info_caption,
            "Base description.|Glowing.|{{FLAM}} The object burns."
        );
    }

    #[test]
    fn menu_info_caption_matches_cpp_buffer_and_line_normalization() {
        let source = format!("A\nB\rC{}", "x".repeat(600));
        let normalized = crate::normalize_menu_info_caption(source);
        assert_eq!(normalized.len(), 512);
        assert!(normalized.starts_with("A B|C"));
    }

    #[test]
    fn contained_context_contents_entry_opens_the_contents_menu() {
        // The C4MN_Context Contents row runs C4CMD_Get with Data=2,
        // which immediately activates C4MN_Contents on the target
        // (C4ObjectMenu.cpp:361-373; C4Command.cpp:1129-1135).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
        lorry.set_category(crate::CATEGORY_VEHICLE);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_test_definition(lorry);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.category = crate::CATEGORY_LIVING;
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        // This assertion exercises C4CMD_Exit's open-door branch. C4Object
        // initializes EntranceStatus to false; HUT3's DOOR script opens it
        // before an object can leave (C4Object.cpp:116;
        // C4Command.cpp:624-650).
        engine.objects[hut_index].state.entrance_status = true;
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(18), "C4MN_Contents");
        assert_eq!(
            menu.items.len(),
            1,
            "contents rows: {:?}",
            menu_item_ids(&menu)
        );
        assert_eq!(menu.items[0].item_id, "LORY");
        assert_eq!(menu.items[0].info_caption, "Carries cargo.");
        assert!(
            menu.items[0]
                .command
                .contains(&format!("\"Activate\", Object({})", lorry.as_u64())),
            "non-carryable vehicles activate out of the base"
        );

        engine.player_in_com(1, COM_THROW, 0).test_value();
        let lorry_after_activate = test_snapshot(&engine, lorry);
        assert_eq!(
            lorry_after_activate.container,
            Some(hut),
            "C4CMD_Activate arms the target's Exit command before it runs"
        );
        assert_eq!(
            lorry_after_activate.command_stack.command_names(),
            vec!["Exit"]
        );
        engine.tick_without_snapshot().test_value();
        let lorry_after_evaluation = test_snapshot(&engine, lorry);
        assert_eq!(
            lorry_after_evaluation.container,
            Some(hut),
            "Exit's InitEvaluation consumes its first object execution"
        );
        assert_eq!(
            lorry_after_evaluation.command_stack.command_names(),
            vec!["Exit"]
        );
        engine.tick_without_snapshot().test_value();
        assert_eq!(
            test_snapshot(&engine, lorry).container,
            None,
            "the vehicle exits on its second object execution"
        );
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(18));
        assert!(menu.items.is_empty());
    }

    #[test]
    fn full_clonk_contents_menu_activates_carryable_rows() {
        // C4MN_Contents downgrades a carryable row to Activate once the menu
        // Clonk reaches its definition CollectionLimit. C4MN_Get does not
        // apply that Contents-only gate (C4ObjectMenu.cpp:300-308).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict 2\nlocal reject_calls;\nprotected func RejectCollect() { reject_calls++; return(0); }\n",
        );
        engine
            .definitions
            .get_mut("CLNK")
            .test_value()
            .set_collection_limit(1);
        let mut hut = structure_definition("HUT3", "Hut", "#strict\n");
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(hut);
        engine.register_test_script_definition("FILL", "Filler", "#strict\n");
        let mut cargo = test_definition("CARG", "Cargo", "#strict\n");
        cargo.set_category(crate::CATEGORY_OBJECT);
        cargo.set_collectible(true);
        engine.register_test_definition(cargo);
        let crew = register_player_crew(&mut engine);
        engine.spawn_test_object(
            SpawnConfig::new("FILL")
                .with_container(crew)
                .with_status(crate::ObjectStatus::Inactive),
        );
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let cargo = engine.spawn_test_object(SpawnConfig::new("CARG").with_container(hut));
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.entrance_status = true;

        let menu = open_contents_test_menu(&mut engine, crew, hut, 18);
        let cargo_row = menu
            .items
            .iter()
            .find(|item| item.item_id == "CARG")
            .test_value();
        assert_eq!(cargo_row.caption, "Activate Cargo");
        assert_eq!(
            cargo_row.command,
            format!(
                "SetCommand(this, \"Activate\", Object({})) && ExecuteCommand()",
                cargo.as_u64()
            )
        );
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(1)),
            "RejectCollect still runs after the limit has already downgraded the row"
        );

        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);
        let menu = open_contents_test_menu(&mut engine, crew, hut, 13);
        let cargo_row = menu
            .items
            .iter()
            .find(|item| item.item_id == "CARG")
            .test_value();
        assert_eq!(cargo_row.caption, "Get Cargo");
        assert!(cargo_row.command.contains("\"Get\""));
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(1)),
            "C4MN_Get skips the Contents-only callback"
        );

        contain_object(&mut engine, crew, hut);
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);
        engine
            .open_container_contents_menu(crew_index, hut_index, 18)
            .test_value();
        let cargo_selection = engine.objects[crew_index]
            .state
            .menu
            .as_ref()
            .expect("Contents menu exists")
            .items
            .iter()
            .position(|item| item.item_id == "CARG")
            .test_value();
        engine.objects[crew_index]
            .state
            .menu
            .as_mut()
            .test_value()
            .selection = i32::try_from(cargo_selection).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();
        let cargo_after = test_snapshot(&engine, cargo);
        assert_eq!(cargo_after.container, Some(hut));
        assert_eq!(
            cargo_after.command_stack.command_names(),
            ["Exit"],
            "the selected row executes C4CMD_Activate on the contained cargo"
        );
        engine.tick_without_snapshot().test_value();
        let cargo_after_evaluation = test_snapshot(&engine, cargo);
        assert_eq!(cargo_after_evaluation.container, Some(hut));
        assert_eq!(
            cargo_after_evaluation.command_stack.command_names(),
            ["Exit"],
            "Exit remains queued after its InitEvaluation frame"
        );
        engine.tick_without_snapshot().test_value();
        assert_eq!(test_snapshot(&engine, cargo).container, None);
    }

    #[test]
    fn contents_refill_calls_reject_collect_once_per_visible_row_and_get_skips_it() {
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            r#"#strict 2
local reject_calls, matching_args, rock_object, flag_object;
protected func RejectCollect(id definition, object item)
{
  reject_calls++;
  if (GetID(item) == definition) matching_args++;
  if (definition == ROCK) rock_object = item;
  if (definition == FLAG) flag_object = item;
  return definition == FLAG;
}
"#,
        );
        let mut hut = structure_definition("HUT3", "Hut", "#strict\n");
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(hut);
        let mut box_definition = test_definition("BOX1", "Box", "#strict\n");
        box_definition.set_category(crate::CATEGORY_VEHICLE);
        box_definition.set_grab_put_get(crate::GRAB_PUT_GET_GET);
        engine.register_test_definition(box_definition);
        for (id, name) in [("ROCK", "Rock"), ("FLAG", "Flag")] {
            let mut definition = test_definition(id, name, "#strict\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            definition.set_collectible(true);
            engine.register_test_definition(definition);
        }
        let mut no_get = test_definition("NGET", "Hidden", "#strict\n");
        no_get.set_category(crate::CATEGORY_OBJECT);
        no_get.set_collectible(true);
        no_get.set_no_get(true);
        engine.register_test_definition(no_get);
        let crew = register_player_crew(&mut engine);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        engine.spawn_test_object(SpawnConfig::new("ROCK").with_container(hut));
        let rock = engine.spawn_test_object(SpawnConfig::new("ROCK").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        let flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("NGET").with_container(hut));
        let box_target = engine.spawn_test_object(SpawnConfig::new("BOX1"));
        let boxed_flag =
            engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(box_target));
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);

        let menu = open_contents_test_menu(&mut engine, crew, hut, 18);
        assert_eq!(menu.items.len(), 2, "NoGet is not an eligible row");
        let rock_row = menu
            .items
            .iter()
            .find(|item| item.item_id == "ROCK")
            .test_value();
        let flag_row = menu
            .items
            .iter()
            .find(|item| item.item_id == "FLAG")
            .test_value();
        assert_eq!(rock_row.caption, "Get Rock");
        assert!(rock_row.command2.contains("\"Get\""));
        assert_eq!(flag_row.caption, "Activate Flag");
        assert!(flag_row.command2.contains("\"Activate\""));

        let crew_index = engine.test_object_index(crew);
        let locals = &engine.objects[crew_index].state.local_vars;
        assert_eq!(locals.get("reject_calls"), Some(&Value::Int(2)));
        assert_eq!(locals.get("matching_args"), Some(&Value::Int(2)));
        assert_eq!(
            locals.get("rock_object"),
            Some(&compat::object_reference_value(rock))
        );
        assert_eq!(
            locals.get("flag_object"),
            Some(&compat::object_reference_value(flag))
        );

        engine.execute_player_controls().test_value();
        let crew_index = engine.test_object_index(crew);
        let locals = &engine.objects[crew_index].state.local_vars;
        assert_eq!(locals.get("reject_calls"), Some(&Value::Int(4)));
        assert_eq!(locals.get("matching_args"), Some(&Value::Int(4)));

        let hut_index = engine.test_object_index(hut);
        engine
            .open_container_contents_menu(crew_index, hut_index, 13)
            .test_value();
        engine.execute_player_controls().test_value();
        let crew_index = engine.test_object_index(crew);
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(4)),
            "C4MN_Get never calls RejectCollect"
        );
        let menu = test_menu(&engine, crew);
        assert!(menu.items.iter().all(|item| {
            item.caption.starts_with("Get ")
                && item.command.contains("\"Get\"")
                && item.command2.contains("\"Get\"")
        }));

        let box_index = engine.test_object_index(box_target);
        let menu = open_contents_test_menu(&mut engine, crew, box_target, 18);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Get Flag");
        assert!(menu.items[0]
            .command
            .contains(&format!("\"Get\", Object({})", boxed_flag.as_u64())));
        let crew_index = engine.test_object_index(crew);
        assert_eq!(
            object_local(&engine, crew, "reject_calls"),
            Some(&Value::Int(5)),
            "RejectCollect runs before a missing Entrance forces the row back to Get"
        );
    }

    #[test]
    fn contents_refill_preserves_the_selected_definition() {
        // C4ObjectMenu::Refill stores the selected item's C4ID and
        // checkIDSelection restores it after rebuilding the rows
        // (C4ObjectMenu.cpp:274,325,448-458).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        for (id, name) in [("LORY", "Lorry"), ("FLAG", "Flag")] {
            let mut definition = test_definition(id, name, "#strict\n");
            definition.set_category(crate::CATEGORY_VEHICLE);
            engine.register_test_definition(definition);
        }
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        test_object_mut(&mut engine, hut).state.base = 1;
        engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        let selected_definition = test_menu(&engine, crew).items[1].item_id.clone();

        engine.execute_player_controls().test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, selected_definition);
    }

    #[test]
    fn contained_context_sell_entry_opens_the_grouped_sell_menu() {
        // The C4MN_Context Sell row runs a data-less C4CMD_Sell and opens
        // C4MN_Sell. Refill walks the base's stContents order, groups
        // equal definitions, and carries both preferred-object and bulk
        // commands (C4ObjectMenu.cpp:238-277; C4Command.cpp:2040-2057).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        let mut flag = test_definition("FLAG", "Flag", "#strict\n");
        flag.set_category(crate::CATEGORY_OBJECT);
        flag.set_value(100);
        flag.set_description(Some("Marks a base.".to_string()));
        engine.register_test_definition(flag);
        let mut lorry = test_definition("LORY", "Lorry", "#strict\n");
        lorry.set_category(crate::CATEGORY_VEHICLE);
        lorry.set_value(20);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_test_definition(lorry);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.category = crate::CATEGORY_LIVING;
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.owner = 8;
        engine.objects[hut_index].state.base = 1;
        let first_flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        let second_flag = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(5), "C4MN_Sell");
        assert_eq!(
            menu.title_symbol,
            crate::ObjectMenuSymbol::Sell { owner: 8 },
            "C4Object::ActivateMenu composes C4MN_Sell with pTarget->Owner (C4Object.cpp:1932-1941; C4Menu.cpp:43-70)"
        );
        assert_eq!(
            menu.extra,
            crate::ObjectMenuExtra::Value,
            "C4MN_Sell enables C4MN_Extra_Value (C4Object.cpp:1938; C4Menu.cpp:843-907)"
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count, item.value))
                .collect::<Vec<_>>(),
            vec![("FLAG", 2, Some(100)), ("LORY", 1, Some(20))]
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.info_caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Marks a base.", "Carries cargo."]
        );
        assert!(
            menu.items[0]
                .command
                .contains(&format!("Object({})", second_flag.as_u64()))
                || menu.items[0]
                    .command
                    .contains(&format!("Object({})", first_flag.as_u64()))
        );
        assert!(menu.items[0].command2.contains(",2,0,,0,FLAG"));
        assert!(menu.items[1]
            .command
            .contains(&format!("Object({})", lorry.as_u64())));

        engine.player_in_com(1, COM_THROW, 0).test_value();
        assert_eq!(engine.player(1).expect("player").wealth(), 100);
        assert_eq!(
            engine
                .player(1)
                .expect("player")
                .home_base_material()
                .get("FLAG"),
            None,
            "a non-Rebuyable definition does not create a missing stock row"
        );
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(5));
        assert_eq!(menu_item_counts(&menu), vec![("FLAG", 1), ("LORY", 1)]);
    }

    #[test]
    fn sell_menu_row_value_uses_object_get_value_and_skips_no_sell() {
        let mut engine = Engine::new();
        let base_script = r#"#strict 2
local sell_value_calls, no_sell_value_calls;
protected func CalcSellValue(object item, int value)
{
    sell_value_calls = sell_value_calls + 1;
    return value + 3;
}
public func MarkNoSellValue()
{
    no_sell_value_calls = no_sell_value_calls + 1;
    return true;
}
"#;
        let (crew, hut) = contained_base_fixture_with_script(&mut engine, 2, base_script);
        let item_script = r#"#strict 2
local value_calls;
protected func CalcValue(object base, int player)
{
    value_calls = value_calls + 1;
    if (!base || player != 1) return 900;
    return 41;
}
"#;
        let mut item = test_definition("ITEM", "Item", item_script);
        item.set_category(crate::CATEGORY_OBJECT);
        item.set_value(99);
        engine.register_test_definition(item);
        let no_sell_script = r#"#strict 2
protected func CalcValue(object base, int player)
{
    base->MarkNoSellValue();
    return 500;
}
"#;
        let mut no_sell = test_definition("NOSL", "No sell", no_sell_script);
        no_sell.set_category(crate::CATEGORY_OBJECT);
        no_sell.set_no_sell(-2);
        engine.register_test_definition(no_sell);
        let item = engine.spawn_test_object(
            SpawnConfig::new("ITEM")
                .with_construction(crate::FULL_CON / 2)
                .with_container(hut),
        );
        engine.spawn_test_object(SpawnConfig::new("NOSL").with_container(hut));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.category = 0;
        let hut_index = engine.test_object_index(hut);

        engine
            .open_base_sell_menu(crew_index, hut_index)
            .test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(
            menu.items
                .iter()
                .map(|row| (row.item_id.as_str(), row.value))
                .collect::<Vec<_>>(),
            vec![("ITEM", Some(23))],
            "41 is construction-scaled to 20 before CalcSellValue adds 3"
        );

        engine
            .refill_base_sell_menu(crew_index, hut_index)
            .test_value();
        assert_eq!(test_menu(&engine, crew).items[0].value, Some(23));
        assert_eq!(
            test_snapshot(&engine, item).local_vars.get("value_calls"),
            Some(&Value::Int(2)),
            "CalcValue reruns on every refill"
        );
        let base = test_snapshot(&engine, hut);
        assert_eq!(
            base.local_vars.get("sell_value_calls"),
            Some(&Value::Int(2))
        );
        assert_eq!(
            base.local_vars.get("no_sell_value_calls"),
            Some(&Value::Nil),
            "NoSell skips the row before invoking CalcValue"
        );
    }

    #[test]
    fn contents_and_sell_refills_group_only_cpp_concatable_pictures() {
        // C4MN_Sell and C4MN_Contents both enumerate the target's stContents
        // through C4ObjectListIterator (C4ObjectMenu.cpp:238-275,279-326).
        // That iterator emits a separate row for same-ID objects unless
        // C4Object::CanConcatPictureWith succeeds (C4ObjectList.cpp:849-903;
        // C4Object.cpp:6173-6213). The row count is the concat group count,
        // while command2 deliberately keeps Contents.ObjectCount(id), i.e. the
        // count of every same-ID object (C4ObjectMenu.cpp:266-271,317-321).
        let mut engine = clonk_engine("#strict\n");
        let mut hut = structure_definition("HUT3", "Hut", "#strict\n");
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_test_definition(hut);
        let mut flint = test_definition("TFLN", "T-Flint", "#strict\n");
        flint.set_category(crate::CATEGORY_OBJECT);
        flint.set_collectible(true);
        flint.set_value(15);
        engine.register_test_definition(flint);
        let crew = register_player_crew(&mut engine);
        let crew_index = engine.test_object_index(crew);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        let idle = engine.spawn_test_object(SpawnConfig::new("TFLN").with_container(hut));
        let activated = engine.spawn_test_object(SpawnConfig::new("TFLN").with_container(hut));
        set_second_picture_row(&mut engine, activated);
        assert!(!engine.can_concat_picture_with(
            &test_snapshot(&engine, idle),
            &test_snapshot(&engine, activated),
        ));

        engine
            .open_base_sell_menu(crew_index, hut_index)
            .test_value();
        let sell = test_menu(&engine, crew);
        assert_eq!(
            sell.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count))
                .collect::<Vec<_>>(),
            vec![("TFLN", 1), ("TFLN", 1)],
            "different per-object pictures occupy separate C++ menu rows"
        );
        assert!(sell
            .items
            .iter()
            .all(|item| item.command2.contains(",2,0,,0,TFLN")));
        let ordered_flints = test_snapshot(&engine, hut).contents;
        assert_eq!(ordered_flints.len(), 2);
        assert_eq!(
            sell.items
                .iter()
                .map(|item| item.picture_object)
                .collect::<Vec<_>>(),
            ordered_flints.iter().copied().map(Some).collect::<Vec<_>>(),
            "C4ObjectMenu draws each row from the representative returned by C4ObjectListIterator (C4ObjectMenu.cpp:246-264; C4ObjectList.cpp:849-903)"
        );
        for (row, object) in sell.items.iter().zip(&ordered_flints) {
            assert!(row
                .command
                .contains(&format!("Object({})", object.as_u64())));
        }
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine
            .open_base_sell_menu(crew_index, hut_index)
            .test_value();
        assert_eq!(
            test_menu(&engine, crew).selection,
            1,
            "same-ID picture rows keep C++'s surviving numeric selection"
        );

        let contents = open_contents_test_menu(&mut engine, crew, hut, 18);
        assert_eq!(
            contents
                .items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count))
                .collect::<Vec<_>>(),
            vec![("TFLN", 1), ("TFLN", 1)]
        );
        assert_eq!(
            contents
                .items
                .iter()
                .map(|item| item.picture_object)
                .collect::<Vec<_>>(),
            ordered_flints.iter().copied().map(Some).collect::<Vec<_>>(),
            "C4ObjectMenu calls Picture2Facet on each Get/Contents representative (C4ObjectMenu.cpp:286-313)"
        );
        assert!(contents.items.iter().all(|item| {
            item.command2.contains(&format!(
                "SetCommand(this, \"Get\", , 2,0, Object({}), TFLN)",
                hut.as_u64()
            ))
        }));
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine
            .open_container_contents_menu(crew_index, hut_index, 18)
            .test_value();
        assert_eq!(test_menu(&engine, crew).selection, 1);
    }

    #[test]
    fn sell_refill_prefers_a_full_construction_picture_representative() {
        // After C4ObjectListIterator fixes the row count, C4ObjectMenu replaces
        // an incomplete representative with the first full-construction object
        // only when their pictures concatenate. The replacement supplies both
        // Picture2Facet and the primary Sell command target; the count remains
        // the original concat-group count (C4ObjectMenu.cpp:246-271).
        let mut engine = clonk_engine("#strict\n");
        let hut = structure_definition("HUT3", "Hut", "#strict\n");
        engine.register_test_definition(hut);
        let mut flint = test_definition("TFLN", "T-Flint", "#strict\n");
        flint.set_category(crate::CATEGORY_OBJECT);
        flint.set_value(15);
        engine.register_test_definition(flint);
        let crew = register_player_crew(&mut engine);
        let crew_index = engine.test_object_index(crew);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        let full = engine.spawn_test_object(SpawnConfig::new("TFLN").with_container(hut));
        let incomplete = engine.spawn_test_object(
            SpawnConfig::new("TFLN")
                .with_construction(crate::FULL_CON / 2)
                .with_container(hut),
        );
        assert_eq!(
            test_snapshot(&engine, hut).contents,
            vec![incomplete, full],
            "the incomplete object is the iterator's initial representative"
        );

        engine
            .open_base_sell_menu(crew_index, hut_index)
            .test_value();
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].count, 2);
        assert_eq!(menu.items[0].picture_object, Some(full));
        assert!(menu.items[0]
            .command
            .contains(&format!("Object({})", full.as_u64())));
    }

    #[test]
    fn sell_refill_preserves_the_selected_definition_and_numeric_fallback() {
        // C4ObjectMenu's C4MN_Sell refill remembers the selected C4ID. If
        // that definition remains, checkIDSelection restores its row; if it
        // disappears, C4Menu::AdjustSelection keeps the old numeric slot
        // when that slot is still valid (C4ObjectMenu.cpp:147-164,238-275;
        // C4Menu.cpp:943-973,993-1017).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        for (id, name, category, value) in [
            ("FLAG", "Flag", crate::CATEGORY_OBJECT, 100),
            ("LORY", "Lorry", crate::CATEGORY_VEHICLE, 20),
            ("BARL", "Barrel", crate::CATEGORY_STRUCTURE, 5),
        ] {
            let mut definition = test_definition(id, name, "#strict\n");
            definition.set_category(category);
            definition.set_value(value);
            engine.register_test_definition(definition);
        }
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.category = crate::CATEGORY_LIVING;
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        test_object_mut(&mut engine, hut).state.base = 1;
        engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("LORY").with_container(hut));
        engine.spawn_test_object(SpawnConfig::new("BARL").with_container(hut));
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();
        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "LORY");
        assert_eq!(menu.items[1].count, 1);

        engine.player_in_com(1, COM_THROW, 0).test_value();

        let menu = test_menu(&engine, crew);
        assert_eq!(menu_item_ids(&menu), vec!["FLAG", "BARL"]);
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "BARL");
    }

    #[test]
    fn closing_auto_context_menu_exits_the_building() {
        // AutoContextMenu installs a close command that issues Exit for
        // selected clonks, then calls ExecuteCommand once. That first call
        // is Exit's InitEvaluation; the next object tick performs the exit
        // (C4Object.cpp:2044-2062; C4Menu.cpp:317-331;
        // C4Command.cpp:1554-1555,1654-1657).
        let mut engine = clonk_engine("#strict\n");
        register_auto_context_structure(&mut engine, "HUT3", "Hut", "#strict\n");
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine.player_mut(1).test_value().control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT3"));
        let hut_index = engine.test_object_index(hut);
        engine.objects[hut_index].state.base = 1;
        // Model the already-open HUT3 door. With EntranceStatus=false C++
        // asks ActivateEntrance and leaves Exit pending instead
        // (C4Command.cpp:624-665).
        engine.objects[hut_index].state.entrance_status = true;
        contain_object(&mut engine, crew, hut);
        engine.execute_player_controls().test_value();

        engine.player_in_com(1, COM_DIG, 0).test_value();

        let crew_snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            crew_snapshot.container,
            Some(hut),
            "the close command's synchronous ExecuteCommand only evaluates Exit"
        );
        assert_eq!(
            crew_snapshot.command_stack.command_names(),
            vec!["Exit".to_string()]
        );
        assert_eq!(
            engine.debug_object_menu(crew.as_u64()),
            Some(None),
            "the context menu remains closed"
        );

        engine.tick_without_snapshot().test_value();
        assert_eq!(
            test_snapshot(&engine, crew).container,
            None,
            "the evaluated context close command exits on the next tick"
        );
    }

    #[test]
    fn contained_com_dig_opens_the_base_sell_menu() {
        // ContainedControl COM_Dig (C4Object.cpp:3275-3280): the sell menu
        // twin, gated on BASEFUNC_Sell.
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);

        engine.player_in_com(1, COM_DIG, 0).test_value();
        assert_eq!(
            test_menu(&engine, crew).identification,
            Value::Int(5),
            "COM_Dig activates C4MN_Sell on the clonk"
        );
        assert!(engine.pending_menu_requests.is_empty());
        assert_eq!(test_snapshot(&engine, crew).container, Some(hut));
    }

    #[test]
    fn contents_count_change_refills_sell_menu_before_tick_35() {
        // C4ObjectMenu::Execute marks every RefillObject menu dirty as soon
        // as the target's total contents count changes, independently of the
        // shared 35-tick timer (C4ObjectMenu.cpp:448-459).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut item = test_definition("ITEM", "Item", "#strict\n");
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        let mut item_two = test_definition("ITM2", "Second Item", "#strict\n");
        item_two.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item_two);
        engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
        let crew_index = engine.test_object_index(crew);
        let hut_index = engine.test_object_index(hut);
        engine
            .open_base_sell_menu(crew_index, hut_index)
            .test_value();
        let initial_location_generation = test_menu(&engine, crew).location_reset_generation;
        assert_eq!(test_menu(&engine, crew).items[0].count, 1);
        // The first ordinary frame observes SetRefillObject's zero-valued
        // cache. Add the second object only after that count is established.
        engine.tick_without_snapshot().test_value();
        assert_eq!(engine.frame(), 1);
        engine.spawn_test_object(SpawnConfig::new("ITM2").with_container(hut));

        engine.execute_player_controls().test_value();
        assert_eq!(engine.frame(), 1, "the immediate refill advances no frame");
        let menu = test_menu(&engine, crew);
        assert_eq!(menu.identification, Value::Int(5));
        let item = menu
            .items
            .iter()
            .find(|item| item.item_id == "ITEM")
            .test_value();
        assert_eq!(item.count, 1);
        assert_eq!(
            menu.items
                .iter()
                .find(|item| item.item_id == "ITM2")
                .test_value()
                .count,
            1
        );
        assert_eq!(menu.items.len(), 3);
        assert_eq!(
            menu.location_reset_generation,
            initial_location_generation.wrapping_add(1),
            "C4ObjectMenu::RefillInternal marks a growing refill (C4Menu.cpp:947-970)",
        );
        assert_eq!(
            menu.refill_object_contents_count, 3,
            "the cache includes the contained crew and both sale objects"
        );
    }

    #[test]
    fn hostile_or_disabled_bases_never_open_buy_menus() {
        // Hostile(Owner, Contained->Base) vetoes (C4Object.cpp:3271), as
        // does a cleared BASEFUNC_Buy bit (:3272).
        let mut engine = Engine::new();
        let (_, _) = contained_base_fixture(&mut engine, 2);
        engine.set_hostility(1, 2, true).test_value();
        engine.player_in_com(1, COM_UP, 0).test_value();
        assert!(
            engine.pending_menu_requests.is_empty(),
            "hostile bases sell nothing"
        );

        let mut engine = Engine::new();
        let (_, _) = contained_base_fixture(&mut engine, 1);
        engine.set_base_buy_enabled(false);
        engine.player_in_com(1, COM_UP, 0).test_value();
        assert!(
            engine.pending_menu_requests.is_empty(),
            "BASEFUNC_Buy off keeps the menu closed"
        );
    }

    #[test]
    fn contained_script_override_beats_hardcoded_exit() {
        // fCallSfEarly containers run Contained<Com> first; a truthy result
        // consumes the com (C4Object.cpp:3239-3251).
        let hut = r#"
#strict
protected func ContainedDown(pByClonk) { return(1); }
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut hut_def = test_definition("HUT1", "Hut", hut);
        hut_def.set_version([4, 9, 1, 3, 0]);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_DOWN, 0).test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the container consumed the com"
        );
    }

    #[test]
    fn old_contained_script_runs_after_hardcoded_exit_and_cannot_consume_it() {
        // Before 4.9.1.3 C4Object::ContainedControl queues its hardcoded
        // action first, then calls Contained<Com> and ignores its return
        // value (src/C4Object.cpp:3246-3316).
        let hut = r#"
#strict
protected func ContainedDown(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut hut_def = test_definition("HUT1", "Hut", hut);
        hut_def.set_version([4, 9, 1, 2, 0]);
        engine.register_test_definition(hut_def);
        let (crew, hut) = contain_player_crew(&mut engine, "HUT1");

        engine.player_in_com(1, COM_DOWN, 0).test_value();

        assert_eq!(test_snapshot(&engine, hut).damage, 1);
        assert_eq!(
            test_snapshot(&engine, crew).command_stack.command_names(),
            vec!["Exit"],
            "the truthy late callback cannot consume the already-queued exit"
        );
    }

    /// Crew with three contents of distinct defs: front ROCK, then GOLD,
    /// then SKUL (front = `contents[0]`, the C4ObjectList First).
    fn wheel_fixture(engine: &mut Engine, clonk_script: &str) -> (ObjectId, [ObjectId; 3]) {
        register_clonk(engine, "CLNK", clonk_script);
        for id in ["ROCK", "GOLD", "SKUL"] {
            let def = test_definition(id, id, "#strict\n");
            engine.register_test_definition(def);
        }
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let crew = spawn_crew(engine, "CLNK", 1);
        let items = ["ROCK", "GOLD", "SKUL"]
            .map(|id| engine.spawn_test_object(SpawnConfig::new(id).with_container(crew)));
        let index = engine.test_object_index(crew);
        engine.objects[index].state.contents = items.to_vec();
        (crew, items)
    }

    fn contents(engine: &Engine, id: ObjectId) -> Vec<ObjectId> {
        let index = engine.test_object_index(id);
        engine.objects[index].state.contents.clone()
    }

    #[test]
    fn enter_then_target_shift_in_one_callback_selects_the_entered_object() {
        // C++ Enter mutates both the child's Contained pointer and the
        // container's raw Contents links before returning. A following
        // ShiftContents in the same callback therefore sees and cyclically
        // relinks the just-entered object (C4Object.cpp:1572-1642,
        // 5816-5836; C4ObjectList.cpp:815-831). Eke's retained pistol uses
        // this exact sequence when it is drawn from the HUD.
        let script = r#"
#strict
public func Redraw(object item)
{
  Enter(this(), item);
  ShiftContents(0, true, GetID(item));
  return(1);
}
"#;
        let mut engine = clonk_engine(script);
        for id in ["ROCK", "GOLD"] {
            let mut definition = test_definition(id, id, "#strict\n");
            definition.set_category(crate::CATEGORY_OBJECT);
            engine.register_test_definition(definition);
        }
        let mut pistol = test_definition("PSTL", "Pistol", "#strict\n");
        pistol.set_category(crate::CATEGORY_STATIC_BACK);
        engine.register_test_definition(pistol);
        let holder = structure_definition("HOLD", "Holder", "#strict\n");
        engine.register_test_definition(holder);
        engine.register_test_player(PlayerConfig::new(1, "Test"));

        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let rock = engine.spawn_test_object(SpawnConfig::new("ROCK").with_container(crew));
        let gold = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(crew));
        let holder = engine.spawn_test_object(SpawnConfig::new("HOLD"));
        let pistol = engine.spawn_test_object(SpawnConfig::new("PSTL").with_container(holder));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.contents = vec![rock, gold];

        assert_eq!(
            engine
                .call_object_function(crew_index, "Redraw", vec![Value::Object(pistol.as_u64())],)
                .expect("redraw callback completes"),
            Value::Int(1)
        );
        assert_eq!(test_snapshot(&engine, pistol).container, Some(crew));
        assert_eq!(
            contents(&engine, crew),
            vec![pistol, rock, gold],
            "the callback-final raw list matches synchronous C++ ordering"
        );
        assert!(
            !contents(&engine, holder).contains(&pistol),
            "Enter unlinks the pistol from its previous container"
        );
    }

    #[test]
    fn wheel_down_shifts_contents_to_the_next_different_item() {
        // COM_WheelDown → ShiftContents(false, true) (C4Object.cpp:
        // 3391-3396): walk First->Next for the first item of a DIFFERENT
        // definition and rotate it to the front (C4Object.cpp:5751-5775).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).test_value();
        assert_eq!(contents(&engine, crew), vec![gold, skul, rock]);
    }

    #[test]
    fn shift_contents_uses_the_first_present_link_as_front() {
        // C4ObjectList::GetObject skips Status=0 links. Both ShiftContents'
        // comparison seed and DirectComContents' already-front check use that
        // first PRESENT object rather than the raw First link
        // (C4Object.cpp:5753,5782; C4ObjectList.cpp:296-308).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");
        test_object_mut(&mut engine, skul).state.status = crate::ObjectStatus::Deleted;
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.contents = vec![skul, rock, gold];

        engine
            .object_direct_com_contents(crew_index, rock, false)
            .test_value();
        assert_eq!(
            contents(&engine, crew),
            vec![skul, rock, gold],
            "the first present object is already front despite a deleted raw link"
        );

        assert!(
            engine
                .object_shift_contents(crew_index, false, false)
                .expect("picture shift succeeds"),
            "the distinct present item is found after the effective front"
        );
        assert_eq!(
            contents(&engine, crew),
            vec![gold, skul, rock],
            "the dead raw link participates in relinking but never seeds comparison"
        );
    }

    #[test]
    fn wheel_shift_separates_same_definition_pictures_that_cannot_concat() {
        // ShiftContents does not merely compare definition IDs: it advances
        // to the first item for which C4Object::CanConcatPictureWith is false
        // (C4Object.cpp:5751-5775,6173-6213). Different ColorMod values split
        // an otherwise identical stack when APS_Color is not enabled.
        let mut engine = clonk_engine("#strict\n");
        engine.register_test_script_definition("ROCK", "Rock", "#strict\n");
        let crew = register_player_crew(&mut engine);
        let plain = engine.spawn_test_object(SpawnConfig::new("ROCK").with_container(crew));
        let tinted = engine.spawn_test_object(
            SpawnConfig::new("ROCK")
                .with_container(crew)
                .with_color_modulation(0x0080_8080),
        );
        test_object_mut(&mut engine, crew).state.contents = vec![plain, tinted];
        engine
            .apply_object_update(
                tinted,
                crate::ObjectUpdate::new().with_status(crate::ObjectStatus::Inactive),
            )
            .test_value();
        assert_eq!(
            test_snapshot(&engine, tinted).status,
            crate::ObjectStatus::Inactive,
            "C++ Status=2 remains truthy for ShiftContents and DirectComContents"
        );

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).test_value();
        assert_eq!(
            contents(&engine, crew),
            vec![tinted, plain],
            "inactive non-concatenable same-definition picture becomes the new front"
        );
    }

    #[test]
    fn wheel_up_shifts_contents_back_to_the_last_different_item() {
        // COM_WheelUp → ShiftContents(true, true): walk from Contents.Last
        // backwards (C4Object.cpp:5757).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine.player_in_com(1, COM_WHEEL_UP, 0).test_value();
        assert_eq!(contents(&engine, crew), vec![skul, rock, gold]);
    }

    #[test]
    fn wheel_shift_respects_the_control_contents_veto() {
        // DirectComContents runs ~ControlContents(id) first; a truthy
        // return takes over the selection (C4Object.cpp:5784-5786).
        let script = r#"
#strict
protected func ControlContents(idTarget) { return(1); }
"#;
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, script);

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).test_value();
        assert_eq!(
            contents(&engine, crew),
            vec![rock, gold, skul],
            "the container's ControlContents consumed the shift"
        );
    }

    #[test]
    fn com_contents_shifts_the_target_to_the_front_of_its_container() {
        // COM_Contents carries the target's object NUMBER in iData and the
        // shift runs on the target's CONTAINER (C4Object.cpp:3364-3372 ->
        // DirectComContents, :5777-5797).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine
            .player_in_com(1, COM_CONTENTS, skul.as_u64() as i32)
            .test_value();
        assert_eq!(contents(&engine, crew), vec![skul, rock, gold]);
        // The new front had no ~Selection handler: the Grab sound plays at
        // the container (C4Object.cpp:5790).
        assert!(
            engine.pending_audio.iter().any(|command| matches!(
                command,
                crate::AudioCommand::PlaySound { name, target, .. }
                    if name == "Grab" && *target == Some(crew)
            )),
            "falsy Selection plays the Grab sound"
        );
    }

    /// Three equal-definition crew members for cursor-com cycling. C++
    /// stMain order is newest-first, while the cursor is deliberately put on
    /// the oldest member to exercise both link walking and wrap-around.
    fn crew_trio(engine: &mut Engine) -> [ObjectId; 3] {
        register_clonk(engine, "CLNK", "#strict\n");
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let spawn = |engine: &mut Engine| {
            engine.spawn_test_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Walk")),
            )
        };
        let trio = [spawn(engine), spawn(engine), spawn(engine)];
        engine.select_crew(1, vec![trio[0]]).test_value();
        engine.set_crew_cursor(1, Some(trio[0])).test_value();
        trio
    }

    fn set_crew_rank(engine: &mut Engine, crew: ObjectId, rank: i32) {
        Rc::make_mut(&mut engine.crew_ranks).insert(crew.as_u64(), rank);
    }

    #[test]
    fn hi_rank_active_crew_cursor_death_prefers_an_older_higher_rank() {
        // Crew is newest-first [head, veteran, cursor]. Once the cursor dies,
        // the fallback pass must scan past the rank-0 head to the older
        // rank-3 veteran (C4Player.cpp:1003-1020,1235-1258).
        let mut engine = Engine::new();
        let [cursor, veteran, head] = crew_trio(&mut engine);
        set_crew_rank(&mut engine, cursor, 0);
        set_crew_rank(&mut engine, veteran, 3);
        set_crew_rank(&mut engine, head, 0);

        let cursor_index = engine.test_object_index(cursor);
        engine.objects[cursor_index].state.alive = true;
        engine.assign_death(cursor_index, false).test_value();

        assert_eq!(engine.crew_cursor(1), Some(veteran));
        assert_eq!(engine.selected_crew(1), vec![veteran]);
    }

    #[test]
    fn hi_rank_active_crew_equal_ranks_keep_the_first_roster_entry() {
        // C4ObjectList::stMain order is newest-first. C++ replaces its
        // candidate only for a STRICTLY higher rank, so equal ranks retain
        // the newest crew member at the head of the roster.
        let mut engine = Engine::new();
        let [oldest, middle, newest] = crew_trio(&mut engine);
        for crew in [oldest, middle, newest] {
            set_crew_rank(&mut engine, crew, 2);
        }

        engine.select_crew(1, [middle, newest]).test_value();

        assert_eq!(engine.player_hi_rank_active_crew(1, false), Some(newest));
        assert_eq!(engine.crew_cursor(1), Some(newest));
    }

    #[test]
    fn hi_rank_active_crew_select_only_ranks_only_selected_members() {
        // The first AdjustCursorCommand pass considers Select members only:
        // the unselected rank-9 middle member cannot beat the selected
        // rank-3 head, which in turn beats the selected rank-1 old cursor.
        let mut engine = Engine::new();
        let [oldest, unselected_veteran, selected_head] = crew_trio(&mut engine);
        set_crew_rank(&mut engine, oldest, 1);
        set_crew_rank(&mut engine, unselected_veteran, 9);
        set_crew_rank(&mut engine, selected_head, 3);

        engine.select_crew(1, [selected_head]).test_value();

        assert_eq!(
            engine.player_hi_rank_active_crew(1, true),
            Some(selected_head)
        );
        assert_eq!(
            engine.player_hi_rank_active_crew(1, false),
            Some(unselected_veteran)
        );
        assert_eq!(engine.crew_cursor(1), Some(selected_head));
    }

    fn control_state(engine: &Engine, owner: i32) -> &crate::player::PlayerControlState {
        &engine.players.get(&owner).test_value().control
    }

    #[test]
    fn cursor_right_cycles_the_crew_in_roster_order_skipping_disabled() {
        // C4Player::CursorRight (C4Player.cpp:1261-1275): the next crew
        // link with Status and !CrewDisabled becomes the cursor;
        // CursorFlash = 30 and CursorSelection = 1.
        let mut engine = Engine::new();
        let [_, b, c] = crew_trio(&mut engine);
        test_object_mut(&mut engine, b).state.crew_disabled = true;

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).test_value();
        assert_eq!(
            engine.crew_cursor(1),
            Some(c),
            "the disabled middle crew is skipped (C4Player.cpp:1267)"
        );
        assert_eq!(control_state(&engine, 1).cursor_flash, 30);
        assert_eq!(control_state(&engine, 1).cursor_selection, 1);
    }

    #[test]
    fn mouse_free_right_click_selects_only_the_next_crew() {
        // C4MouseControl::SendPlayerSelectNext queues a one-object
        // CID_PlrSelect, whose C4Player::SelectCrew immediately replaces the
        // whole selection. It is not COM_CursorRight's pending selection mode
        // (C4MouseControl.cpp:1284-1300; C4Control.cpp:341-369).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);
        engine.select_crew(1, [a, b, c]).test_value();
        engine.set_crew_cursor(1, Some(a)).test_value();

        assert!(engine
            .player_mouse_select_next(1)
            .expect("mouse select-next control"));
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(engine.selected_crew(1), vec![c]);
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
        assert_eq!(control_state(&engine, 1).cursor_toggled, 0);
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn mouse_right_drag_frame_skips_disabled_crew_and_replaces_selection() {
        // UpdateCrewSelection compares crew origins against an inclusive
        // frame and CID_PlrSelect executes C4Player::SelectCrew, which first
        // unselects the old set (C4MouseControl.cpp:610-624,1160-1171;
        // C4Player.cpp:1848-1862).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);
        for (id, position) in [
            (a, Vector2::new(5, 5)),
            (b, Vector2::new(10, 10)),
            (c, Vector2::new(15, 15)),
        ] {
            let index = engine.test_object_index(id);
            engine.objects[index].state.position = position;
        }
        test_object_mut(&mut engine, b).state.crew_disabled = true;

        assert_eq!(
            engine.mouse_drag_crew_in_rect(1, Vector2::ZERO, Vector2::new(10, 10)),
            vec![a],
            "the max edge is inclusive, but CrewDisabled is not selectable"
        );
        engine.select_crew(1, [a, b, c]).test_value();
        engine.player_mouse_select_crew(1, [c]).test_value();
        assert_eq!(engine.selected_crew(1), vec![c]);
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn player_select_calls_mouse_selection_and_keeps_crew_for_noncrew_packets() {
        let mut engine = Engine::new();
        let [a, b, _] = crew_trio(&mut engine);
        engine.select_crew(1, [a, b]).test_value();
        let before = engine.selected_crew(1);

        let mut selectable = test_definition("PICK", "Selectable", "#strict\nprotected func MouseSelection(player) { if (player == 1) SetCategory(17); return(1); }\n");
        selectable.set_category(CATEGORY_MOUSE_SELECT);
        engine.register_test_definition(selectable);
        let target = engine.spawn_test_object(SpawnConfig::new("PICK"));

        assert!(engine
            .execute_player_select(&PlayerSelectControlData {
                player: 1,
                objects: vec![target.as_u64() as i32],
                by_client: 4,
            })
            .expect("selection packet executes"));

        assert_eq!(
            test_snapshot(&engine, target).category,
            17,
            "MouseSelection receives the packet player before crew filtering"
        );
        assert_eq!(engine.selected_crew(1), before);
        let player = engine.player(1).test_value();
        assert_eq!((player.control_count(), player.action_count()), (1, 1));
    }

    #[test]
    fn player_select_rechecks_status_after_mouse_selection_callback() {
        let mut engine = Engine::new();
        let [old, _, _] = crew_trio(&mut engine);
        engine.select_crew(1, [old]).test_value();
        engine.set_crew_cursor(1, Some(old)).test_value();

        let mut selectable = test_definition("GONE", "Removed selectable", "#strict\nprotected func MouseSelection(player) { if (player == 1) RemoveObject(); return(1); }\n");
        selectable.set_category(CATEGORY_MOUSE_SELECT);
        engine.register_test_definition(selectable);
        let removed = engine.spawn_test_object(
            SpawnConfig::new("GONE")
                .with_owner(1)
                .with_crew_member(true),
        );

        engine
            .execute_player_select(&PlayerSelectControlData {
                player: 1,
                objects: vec![removed.as_u64() as i32],
                by_client: -1,
            })
            .test_value();

        assert_eq!(
            test_snapshot(&engine, removed).status,
            crate::ObjectStatus::Deleted
        );
        assert_eq!(engine.selected_crew(1), vec![old]);
        assert_eq!(engine.crew_cursor(1), Some(old));
    }

    #[test]
    fn empty_player_select_runs_the_complete_deselection_path() {
        let mut engine = Engine::new();
        let crew = crew_trio(&mut engine);
        engine.select_crew(1, crew).test_value();

        engine
            .execute_player_select(&PlayerSelectControlData {
                player: 1,
                objects: Vec::new(),
                by_client: -1,
            })
            .test_value();

        let selected = engine.selected_crew(1);
        assert_eq!(
            selected.len(),
            1,
            "AdjustCursorCommand selects one fallback"
        );
        assert!(crew.contains(&selected[0]));
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
        assert_eq!(control_state(&engine, 1).cursor_toggled, 0);
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn player_select_count_uses_the_ordered_valid_object_checksum() {
        let (mut engine, first, second) = engine_with_counted_crew();
        let expected = [first, second].into_iter().fold(0_i32, |checksum, id| {
            let number = id.as_u64() as i32;
            checksum.wrapping_add(number.wrapping_mul(checksum.wrapping_add(4_787_821)))
        });
        engine.count_player_control(0, CountedControlType::Command, expected, 1);

        engine
            .execute_player_select(&PlayerSelectControlData {
                player: 0,
                objects: vec![first.as_u64() as i32, 999_999, second.as_u64() as i32],
                by_client: 3,
            })
            .test_value();
        let player = engine.player(0).test_value();
        assert_eq!(
            (player.control_count(), player.action_count()),
            (2, 1),
            "invalid numbers are skipped and the valid ordered fold matches C++"
        );

        engine
            .execute_player_select(&PlayerSelectControlData {
                player: 0,
                objects: vec![first.as_u64() as i32],
                by_client: 3,
            })
            .test_value();
        let player = engine.player(0).test_value();
        assert_eq!((player.control_count(), player.action_count()), (3, 2));
    }

    #[test]
    fn mouse_object_frame_uses_main_list_order_and_caps_selection_at_twenty() {
        // UpdateObjectSelection walks Game.Objects.First, adds with stNone,
        // and breaks at 20 (C4MouseControl.cpp:626-645). Same-definition
        // runtime objects are newest-first in that master list.
        let mut engine = Engine::new();
        let mut item = test_definition("ITEM", "Item", "#strict\n");
        item.set_collectible(true);
        engine.register_test_definition(item);
        let items = (0..22)
            .map(|x| {
                engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(x, x)))
            })
            .collect::<Vec<_>>();

        let selected = engine.mouse_drag_carryables_in_rect(Vector2::ZERO, Vector2::new(30, 30));
        assert_eq!(selected.len(), 20);
        assert_eq!(
            selected,
            items[2..].iter().rev().copied().collect::<Vec<_>>()
        );
    }

    #[test]
    fn mouse_carryable_cursor_preserves_throw_direction_and_point() {
        // DragMoving selects Drop within five pixels of ground, no moving
        // command in solid, and Throw when FindThrowingPosition reaches a
        // free-air target (C4MouseControl.cpp:849-878).
        let mut engine = Engine::new();
        let mut landscape = Landscape::flat(100, 50);
        landscape.set_world_height(100);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(100, 12, -20));

        assert_eq!(
            engine.mouse_drag_carryable_cursor(1, Vector2::new(20, 45)),
            Some(MouseDragCarryableCursor::Drop)
        );
        assert_eq!(
            engine.mouse_drag_carryable_cursor(1, Vector2::new(20, 50)),
            None
        );
        assert_eq!(
            engine.mouse_drag_carryable_cursor(1, Vector2::new(70, 20)),
            Some(MouseDragCarryableCursor::Throw {
                direction: 1,
                landing: Vector2::new(64, 49),
            })
        );
        assert_eq!(
            engine.mouse_drag_carryable_cursor(1, Vector2::new(5, 20)),
            Some(MouseDragCarryableCursor::Throw {
                direction: -1,
                landing: Vector2::new(11, 49),
            })
        );
    }

    #[test]
    fn mouse_dragged_objects_queue_set_then_append_in_selection_order() {
        // ButtonUpDragMoving sends C4P_Command_Set for the first selected
        // object and C4P_Command_Append thereafter (C4MouseControl.cpp:
        // 1171-1201; C4Player.cpp:1445-1450).
        let mut engine = Engine::new();
        let (crew, first) = drop_window_fixture(&mut engine);
        let second = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(crew));

        assert!(engine
            .player_mouse_drag_objects(1, CommandId::Drop, [second, first], Vector2::new(25, 30),)
            .expect("mouse object controls execute"));
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "Drop");
        assert_eq!(commands[0].target, Some(second));
        assert_eq!(commands[0].tx, Some(25));
        assert_eq!(commands[0].ty, Some(30));
        assert_eq!(commands[1].name, "Drop");
        assert_eq!(commands[1].target, Some(first));
    }

    #[test]
    fn player_command_packets_resolve_pointers_data_and_stack_modes() {
        let mut engine = Engine::new();
        let (crew, live_target) = drop_window_fixture(&mut engine);
        let inactive_target = engine.spawn_test_object(SpawnConfig::new("GOLD"));
        engine
            .apply_object_update(
                inactive_target,
                crate::ObjectUpdate::new().with_status(crate::ObjectStatus::Inactive),
            )
            .test_value();
        assert_eq!(
            test_snapshot(&engine, inactive_target).status,
            crate::ObjectStatus::Inactive
        );

        engine
            .player_object_command(1, CommandId::Wait, None, 0, 0)
            .test_value();
        engine
            .execute_player_command(
                1,
                CommandId::Get as i32,
                10,
                20,
                live_target.as_u64() as i32,
                inactive_target.as_u64() as i32,
                41,
                C4P_COMMAND_ADD,
            )
            .test_value();
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["Get", "Wait"]
        );
        assert_eq!(commands[0].target, Some(live_target));
        assert_eq!(commands[0].target2, Some(inactive_target));
        assert_eq!(commands[0].data, CommandData::Integer(41));

        engine
            .execute_player_command(
                1,
                CommandId::Drop as i32,
                30,
                40,
                999_999,
                0,
                42,
                C4P_COMMAND_APPEND,
            )
            .test_value();
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["Get", "Wait", "Drop"]
        );
        assert_eq!(
            commands[2].target, None,
            "a missing object number resolves to nil"
        );
        assert_eq!(commands[2].data, CommandData::Integer(42));

        engine
            .execute_player_command(
                1,
                CommandId::MoveTo as i32,
                50,
                60,
                999_999,
                inactive_target.as_u64() as i32,
                43,
                C4P_COMMAND_SET,
            )
            .test_value();
        engine
            .execute_player_command(
                1,
                CommandId::Get as i32,
                0,
                0,
                live_target.as_u64() as i32,
                0,
                44,
                C4P_COMMAND_SET | C4P_COMMAND_APPEND,
            )
            .test_value();
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["MoveTo", "Get"],
            "Set replaces while combined Set|Append follows Append priority"
        );
        assert_eq!(commands[0].target, None);
        assert_eq!(commands[0].target2, Some(inactive_target));
        assert_eq!(commands[0].data, CommandData::Integer(43));
        assert_eq!(commands[1].target, Some(live_target));
        assert_eq!(commands[1].data, CommandData::Integer(44));

        let player = engine.player(1).test_value();
        assert_eq!((player.control_count(), player.action_count()), (4, 4));
    }

    #[test]
    fn player_command_range_filters_selected_crew_relative_to_the_cursor() {
        let mut engine = Engine::new();
        let [cursor, distant, _] = crew_trio(&mut engine);
        engine.select_crew(1, [cursor, distant]).test_value();
        engine.set_crew_cursor(1, Some(cursor)).test_value();
        engine
            .apply_object_update(
                cursor,
                crate::ObjectUpdate::new().with_position(Vector2::new(100, 100)),
            )
            .test_value();
        engine
            .apply_object_update(
                distant,
                crate::ObjectUpdate::new().with_position(Vector2::new(116, 100)),
            )
            .test_value();

        engine
            .execute_player_command(
                1,
                CommandId::Wait as i32,
                0,
                0,
                0,
                0,
                0,
                C4P_COMMAND_SET | C4P_COMMAND_RANGE,
            )
            .test_value();

        assert_eq!(
            test_snapshot(&engine, cursor).command_stack.command_names(),
            ["Wait"],
            "the cursor is always within its own ±15 range"
        );
        assert!(
            test_snapshot(&engine, distant).command_stack.is_empty(),
            "a selected crew member sixteen pixels from the cursor is filtered"
        );
    }

    #[test]
    fn mouse_control_drag_put_targets_container_and_appends_items_in_order() {
        // UpdatePutTarget chooses only OCF_Container objects, then
        // ButtonUpDragMoving sends Put(Target=container, Target2=item): Set
        // for the first item and Append for each following item
        // (C4MouseControl.cpp:742-768,1171-1201).
        let mut engine = Engine::new();
        let (crew, first) = drop_window_fixture(&mut engine);
        let second = engine.spawn_test_object(SpawnConfig::new("GOLD").with_container(crew));
        let mut hut = test_definition("HUT1", "Hut", "#strict\n");
        hut.set_grab_put_get(crate::GRAB_PUT_GET_PUT);
        engine.register_test_definition(hut);
        let hut = engine.spawn_test_object(SpawnConfig::new("HUT1"));

        assert!(engine
            .player_mouse_drag_put(1, [second, first], hut, false)
            .expect("mouse Put controls execute"));
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.name == "Put"));
        assert!(commands.iter().all(|command| command.target == Some(hut)));
        assert_eq!(commands[0].target2, Some(second));
        assert_eq!(commands[1].target2, Some(first));
        assert!(commands.iter().all(|command| command.tx.is_none()));
        assert!(commands.iter().all(|command| command.ty.is_none()));
    }

    #[test]
    fn mouse_vehicle_drag_requires_grab_one_and_carryable_wins() {
        // DragNone starts a landscape vehicle drag only for the Grab/Ungrab
        // cursor and Def->Grab == 1. DragMoving checks OCF_Carryable first,
        // so a hybrid object remains an item drag (C4MouseControl.cpp:
        // 922-941,833-889).
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        test_object_mut(&mut engine, crew).state.position = Vector2::new(100, 100);

        let mut vehicle = test_definition("VEH1", "Vehicle", "#strict\n");
        vehicle.set_grab(1);
        vehicle.set_category(crate::CATEGORY_VEHICLE);
        engine.register_test_definition(vehicle);
        let mut grab_only = test_definition("VEH2", "Grab-only", "#strict\n");
        grab_only.set_grab(2);
        grab_only.set_category(crate::CATEGORY_VEHICLE);
        engine.register_test_definition(grab_only);
        let mut hybrid = test_definition("VEH3", "Hybrid", "#strict\n");
        hybrid.set_grab(1);
        hybrid.set_category(crate::CATEGORY_VEHICLE);
        hybrid.set_collectible(true);
        engine.register_test_definition(hybrid);
        let mut site = test_definition("SITE", "Site", "#strict\n");
        site.set_grab(1);
        site.set_category(crate::CATEGORY_VEHICLE);
        site.set_collectible(true);
        site.set_constructable(true);
        engine.register_test_definition(site);

        let vehicle =
            engine.spawn_test_object(SpawnConfig::new("VEH1").with_position(Vector2::new(10, 10)));
        let grab_only =
            engine.spawn_test_object(SpawnConfig::new("VEH2").with_position(Vector2::new(20, 10)));
        let hybrid =
            engine.spawn_test_object(SpawnConfig::new("VEH3").with_position(Vector2::new(30, 10)));
        let site = engine.spawn_test_object(
            SpawnConfig::new("SITE")
                .with_position(Vector2::new(40, 10))
                .with_construction(crate::FULL_CON / 2),
        );

        assert_eq!(
            engine.mouse_world_drag_source(1, vehicle, Vector2::new(10, 10)),
            Some(crate::MouseDragSource::Vehicle)
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, grab_only, Vector2::new(20, 10)),
            None,
            "Grab=2 has a Grab cursor but cannot enter C4MC_Drag_Moving"
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, hybrid, Vector2::new(30, 10)),
            Some(crate::MouseDragSource::Carryable),
            "OCF_Carryable is evaluated before the vehicle branch"
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, site, Vector2::new(40, 10)),
            None,
            "the later Build cursor overrides Carryable and Grab"
        );
    }

    #[test]
    fn mouse_left_double_dispatches_every_classic_world_cursor() {
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Local"));
        engine.register_test_player(PlayerConfig::new(2, "Enemy"));
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let crew = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(400, 100))
                .with_action(ActionState::new("Walk")),
        );
        engine.select_crew(1, [crew]).test_value();
        engine.set_crew_cursor(1, Some(crew)).test_value();
        engine.set_landscape(Landscape::flat(512, 300));

        let mut entrance = test_definition("ENTR", "Entrance", "#strict\n");
        entrance.set_entrance_rect(Some(crate::DefinitionRect::new(-2, -2, 4, 4)));
        engine.register_test_definition(entrance);

        let mut vehicle = test_definition("VEH1", "Vehicle", "#strict\n");
        vehicle.set_grab(1);
        engine.register_test_definition(vehicle);
        let mut grab_two = test_definition("VEH2", "Grab two", "#strict\n");
        grab_two.set_grab(2);
        engine.register_test_definition(grab_two);

        let mut tree = test_definition("TREE", "Tree", "#strict\n");
        tree.set_shape_rect(Some(crate::DefinitionRect::new(-15, -45, 30, 90)));
        engine.register_test_definition(tree);

        for (id, name) in [
            ("SITE", "Construction site"),
            ("ITEM", "Carryable"),
            ("ENMY", "Enemy"),
            ("EXCL", "Exclusive"),
        ] {
            engine.register_test_script_definition(id, name, "#strict\n");
        }

        let entrance =
            engine.spawn_test_object(SpawnConfig::new("ENTR").with_position(Vector2::new(40, 40)));
        let vehicle =
            engine.spawn_test_object(SpawnConfig::new("VEH1").with_position(Vector2::new(70, 40)));
        let grab_two =
            engine.spawn_test_object(SpawnConfig::new("VEH2").with_position(Vector2::new(100, 40)));
        let tree =
            engine.spawn_test_object(SpawnConfig::new("TREE").with_position(Vector2::new(140, 80)));
        let site =
            engine.spawn_test_object(SpawnConfig::new("SITE").with_position(Vector2::new(190, 40)));
        let item =
            engine.spawn_test_object(SpawnConfig::new("ITEM").with_position(Vector2::new(230, 40)));
        let enemy = engine.spawn_test_object(
            SpawnConfig::new("ENMY")
                .with_owner(2)
                .with_position(Vector2::new(270, 40)),
        );
        let exclusive = engine
            .spawn_test_object(SpawnConfig::new("EXCL").with_position(Vector2::new(310, 320)));

        let set_ocf = |engine: &mut Engine, object, value| {
            let index = engine.test_object_index(object);
            engine.objects[index].state.ocf = value;
        };
        set_ocf(&mut engine, entrance, ocf::CONTAINER | ocf::ENTRANCE);
        set_ocf(&mut engine, vehicle, ocf::GRAB);
        set_ocf(&mut engine, grab_two, ocf::GRAB);
        set_ocf(&mut engine, tree, ocf::CHOP);
        set_ocf(
            &mut engine,
            site,
            ocf::GRAB | ocf::CARRYABLE | ocf::CONSTRUCT,
        );
        set_ocf(&mut engine, item, ocf::CARRYABLE);
        set_ocf(&mut engine, enemy, ocf::ALIVE);
        set_ocf(&mut engine, exclusive, ocf::EXCLUSIVE);
        let enemy_index = engine.test_object_index(enemy);
        engine.objects[enemy_index].state.alive = true;
        engine.objects[enemy_index].state.category = CATEGORY_MOUSE_SELECT;
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.ocf |= ocf::ALIVE;

        let packet = |command: CommandId,
                      x: i32,
                      y: i32,
                      target: Option<ObjectId>,
                      data: i32,
                      shift: bool| PlayerCommandControlData {
            player: 1,
            command: command as i32,
            x,
            y,
            target: target.map_or(0, |target| target.as_u64() as i32),
            target2: 0,
            data,
            add_mode: if shift { 5 } else { 1 },
            by_client: -1,
        };

        let entrance_point = Vector2::new(45, 45);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(entrance), entrance_point, false, false,),
            Some(packet(
                CommandId::Enter,
                entrance_point.x,
                entrance_point.y,
                Some(entrance),
                0,
                false,
            )),
            "cached Entrance keeps a Container enterable outside its small entrance rect",
        );

        let vehicle_point = Vector2::new(70, 40);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(vehicle), vehicle_point, false, false,),
            Some(packet(CommandId::Grab, 0, 0, Some(vehicle), 0, false)),
        );
        engine.objects[crew_index].state.action = ActionState::new("Push");
        engine.objects[crew_index].state.action.target = Some(vehicle);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(vehicle), vehicle_point, false, false,),
            Some(packet(
                CommandId::UnGrab,
                vehicle_point.x,
                vehicle_point.y,
                Some(vehicle),
                0,
                false,
            )),
        );
        engine.objects[crew_index].state.action = ActionState::new("Walk");

        assert_eq!(
            engine
                .mouse_left_double_command(1, Some(grab_two), Vector2::new(100, 40), false, true,),
            Some(packet(CommandId::Grab, 0, 0, Some(grab_two), 0, true)),
            "Grab=2 is still a double-click Grab and Shift appends it",
        );

        let tree_snapshot = test_snapshot(&engine, tree);
        let tree_shape = engine.object_current_shape_rect(tree).test_value();
        let chop_point = Vector2::new(
            tree_snapshot.position.x + tree_shape.width / 3,
            tree_snapshot.position.y + tree_shape.width / 3,
        );
        assert_eq!(
            engine.mouse_left_double_command(1, Some(tree), chop_point, false, false),
            Some(packet(
                CommandId::Chop,
                chop_point.x,
                chop_point.y,
                Some(tree),
                0,
                false,
            )),
            "the reduced Chop cursor zone includes its boundary",
        );
        let outside_chop = Vector2::new(chop_point.x + 1, chop_point.y);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(tree), outside_chop, false, false),
            None,
            "a picked tree outside the reduced Chop zone remains a crosshair",
        );

        let site_point = Vector2::new(190, 40);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(site), site_point, false, false),
            Some(packet(
                CommandId::Build,
                site_point.x,
                site_point.y,
                Some(site),
                0,
                false,
            )),
            "Build overrides both Carryable and Grab",
        );
        assert_eq!(
            engine.mouse_left_double_command(1, Some(item), Vector2::new(230, 40), false, false,),
            Some(packet(CommandId::Get, 0, 0, Some(item), 0, false)),
        );

        engine.set_hostility(1, 2, true).test_value();
        let enemy_point = Vector2::new(270, 40);
        assert_eq!(
            engine.mouse_left_double_command(1, Some(enemy), enemy_point, false, false),
            Some(packet(
                CommandId::Attack,
                enemy_point.x,
                enemy_point.y,
                Some(enemy),
                0,
                false,
            )),
            "Attack overrides MouseSelect",
        );
        engine.set_hostility(1, 2, false).test_value();
        assert_eq!(
            engine.mouse_left_double_command(1, Some(enemy), enemy_point, false, false),
            None,
            "MouseSelect has no LeftDouble command",
        );
        assert_eq!(
            engine.mouse_left_double_command(1, Some(crew), Vector2::new(400, 100), false, false,),
            None,
            "own-crew Select has no LeftDouble command",
        );

        let solid = Vector2::new(20, 320);
        assert_eq!(
            engine.mouse_left_double_command(1, None, solid, false, false),
            Some(packet(CommandId::Dig, solid.x, solid.y, None, 0, false)),
        );
        assert_eq!(
            engine.mouse_left_double_command(1, None, solid, true, false),
            Some(packet(CommandId::Dig, solid.x, solid.y, None, 1, false)),
            "Control selects DigMaterial data",
        );
        assert_eq!(
            engine.mouse_left_double_command(
                1,
                Some(exclusive),
                Vector2::new(310, 320),
                false,
                false,
            ),
            None,
            "an OCF_Exclusive-only pick suppresses landscape Dig",
        );
        assert_eq!(
            engine.mouse_left_double_command(1, Some(item), Vector2::new(408, 85), false, false,),
            None,
            "the last-evaluated Jump cursor suppresses Get",
        );
        assert_eq!(
            engine.mouse_left_double_command(1, None, Vector2::new(20, 20), false, false),
            None,
            "a free-air crosshair has no double-click command",
        );
        assert_eq!(
            engine.mouse_left_double_command(99, None, solid, false, false),
            None,
            "a missing player cannot issue a mouse command",
        );
    }

    #[test]
    fn mouse_jump_zone_matches_classic_bounds_and_cursor_state() {
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.position = Vector2::new(100, 100);

        assert!(
            engine.mouse_jump_zone(1, Vector2::new(108, 85)),
            "+8/-15 is inside the classic jump zone"
        );
        assert_eq!(
            engine.mouse_world_cursor(1, None, Vector2::new(92, 85), false),
            MouseWorldCursor::JumpLeft
        );
        assert_eq!(
            engine.mouse_world_cursor(1, None, Vector2::new(108, 85), false),
            MouseWorldCursor::JumpRight
        );
        for dx in [-15, -1, 1, 15] {
            for dy in [-25, -10] {
                assert!(
                    engine.mouse_jump_zone(1, Vector2::new(100 + dx, 100 + dy)),
                    "inclusive boundary dx={dx}, dy={dy}"
                );
            }
        }
        for dx in [-16, 0, 16] {
            assert!(
                !engine.mouse_jump_zone(1, Vector2::new(100 + dx, 85)),
                "excluded horizontal coordinate dx={dx}"
            );
        }
        for dy in [-26, -9] {
            assert!(
                !engine.mouse_jump_zone(1, Vector2::new(108, 100 + dy)),
                "excluded vertical coordinate dy={dy}"
            );
        }
        assert!(
            !engine.mouse_jump_zone(2, Vector2::new(108, 85)),
            "an unregistered player has no mouse jump cursor"
        );

        engine.objects[crew_index].state.container = Some(ObjectId::new(999));
        assert!(
            !engine.mouse_jump_zone(1, Vector2::new(108, 85)),
            "contained cursor objects cannot use the jump zone"
        );
        engine.objects[crew_index].state.container = None;
        engine.objects[crew_index].state.action = ActionState::new("Jump");
        assert!(
            !engine.mouse_jump_zone(1, Vector2::new(108, 85)),
            "only DFA_WALK cursor objects can use the jump zone"
        );
    }

    #[test]
    fn mouse_jump_zone_still_vetoes_world_drag_source() {
        let mut engine = Engine::new();
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.position = Vector2::new(100, 100);

        let mut vehicle = test_definition("VEH1", "Vehicle", "#strict\n");
        vehicle.set_grab(1);
        vehicle.set_category(crate::CATEGORY_VEHICLE);
        engine.register_test_definition(vehicle);
        let point = Vector2::new(108, 85);
        let vehicle = engine.spawn_test_object(SpawnConfig::new("VEH1").with_position(point));

        assert_eq!(
            engine.mouse_world_drag_source(1, vehicle, point),
            None,
            "the last-evaluated jump cursor keeps vetoing a vehicle drag"
        );
        engine.objects[crew_index].state.action = ActionState::new("Jump");
        assert_eq!(
            engine.mouse_world_drag_source(1, vehicle, point),
            Some(crate::MouseDragSource::Vehicle),
            "without the jump cursor the original vehicle drag remains available"
        );
    }

    #[test]
    fn mouse_right_drag_region_expands_same_id_in_contents_order() {
        // A right drag from a viewport inventory region selects every object
        // with the target's ID in its containing object's forward Contents
        // list; a single/left drag keeps only the region target
        // (C4MouseControl.cpp:942-961).
        let mut engine = Engine::new();
        let mut container = test_definition("CONT", "Container", "#strict\n");
        container.set_grab_put_get(crate::GRAB_PUT_GET_GET);
        engine.register_test_definition(container);
        let mut item = test_definition("ITEM", "Item", "#strict\n");
        item.set_collectible(true);
        engine.register_test_definition(item);
        let mut other = test_definition("OTHR", "Other", "#strict\n");
        other.set_collectible(true);
        engine.register_test_definition(other);

        let container = engine.spawn_test_object(SpawnConfig::new("CONT"));
        let first = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        engine.spawn_test_object(SpawnConfig::new("OTHR").with_container(container));
        let second = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
        let third = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));

        assert_eq!(
            engine.mouse_region_drag_objects(first, false),
            vec![first],
            "non-right region drags keep one object"
        );
        assert_eq!(
            engine.mouse_region_drag_objects(first, true),
            vec![third, second, first],
            "runtime stContents is newest-first inside the same-ID cluster"
        );

        engine
            .apply_object_update(
                first,
                crate::ObjectUpdate::new().with_status(crate::ObjectStatus::Inactive),
            )
            .test_value();
        engine
            .apply_object_update(
                second,
                crate::ObjectUpdate::new().with_status(crate::ObjectStatus::Inactive),
            )
            .test_value();
        assert_eq!(
            engine.mouse_region_drag_source(first),
            Some(crate::MouseDragSource::Carryable),
            "inactive objects retain their C++ pointer and copied OCF"
        );
        assert_eq!(
            engine.mouse_region_drag_objects(first, true),
            vec![third, second, first],
            "inactive same-ID contents remain in the pointer-backed group"
        );

        let first_index = engine.test_object_index(first);
        engine.objects[first_index].state.status = crate::ObjectStatus::Deleted;
        engine.objects[first_index].destroyed = true;
        assert!(
            engine.mouse_region_drag_objects(first, false).is_empty(),
            "ordinary Selection.Add rejects a Status-zero target"
        );
        assert_eq!(
            engine.mouse_region_drag_objects(first, true),
            vec![third, second],
            "ObjectCount and Selection.Add both skip the Status-zero link"
        );
        let original_container = engine.objects[first_index].state.container.take();
        assert!(
            engine.mouse_region_drag_objects(first, true).is_empty(),
            "the no-container right-drag branch still goes through Selection.Add"
        );
        engine.objects[first_index].state.container = original_container;
        let second_index = engine.test_object_index(second);
        engine.objects[second_index].state.status = crate::ObjectStatus::Deleted;
        engine.objects[second_index].destroyed = true;
        assert!(
            engine.mouse_region_drag_objects(first, true).is_empty(),
            "the single-object Selection.Add also rejects a Status-zero target"
        );
    }

    #[test]
    fn mouse_dragged_vehicles_queue_push_to_set_then_append() {
        // ButtonUpDragMoving emits PushTo(Target=vehicle, Target2=putTarget)
        // at the release coordinates. The first command is Set and following
        // vehicles are Append; Shift makes the first Append too
        // (C4MouseControl.cpp:1171-1227).
        let mut engine = clonk_engine("#strict\n");
        let mut vehicle = test_definition("VEH1", "Vehicle", "#strict\n");
        vehicle.set_grab(1);
        engine.register_test_definition(vehicle);
        let mut container = test_definition("CONT", "Container", "#strict\n");
        container.set_grab_put_get(crate::GRAB_PUT_GET_PUT);
        engine.register_test_definition(container);
        let crew = register_player_crew(&mut engine);
        let first = engine.spawn_test_object(SpawnConfig::new("VEH1"));
        let second = engine.spawn_test_object(SpawnConfig::new("VEH1"));
        let destination = engine.spawn_test_object(SpawnConfig::new("CONT"));

        assert!(engine
            .player_mouse_drag_vehicles(
                1,
                [second, first],
                Vector2::new(70, 80),
                Some(destination),
                false,
            )
            .expect("vehicle commands execute"));
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.name == "PushTo"));
        assert_eq!(commands[0].target, Some(second));
        assert_eq!(commands[1].target, Some(first));
        assert!(commands
            .iter()
            .all(|command| command.target2 == Some(destination)));
        assert!(commands
            .iter()
            .all(|command| command.tx == Some(70) && command.ty == Some(80)));

        assert!(engine
            .player_mouse_drag_vehicles(1, [first], Vector2::new(90, 100), None, true,)
            .expect("Shift-append vehicle command executes"));
        let commands = test_snapshot(&engine, crew).command_stack.command_views();
        assert_eq!(commands.len(), 3, "Shift preserves both prior commands");
        assert_eq!(commands[2].name, "PushTo");
        assert_eq!(commands[2].target, Some(first));
        assert_eq!(commands[2].target2, None);
        assert_eq!(commands[2].tx, Some(90));
        assert_eq!(commands[2].ty, Some(100));
    }

    #[test]
    fn cursor_left_steps_to_the_previous_master_order_crew_member() {
        // C4Player::CursorLeft (C4Player.cpp:1278-1293): equal-definition
        // crew links are newest-first, so the member before the oldest is
        // the middle-created Clonk.
        let mut engine = Engine::new();
        let [_, b, _] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_LEFT, 0).test_value();
        assert_eq!(engine.crew_cursor(1), Some(b));
    }

    #[test]
    fn cursor_toggle_in_selection_mode_toggles_the_cursor_select() {
        // After a cursor com CursorSelection = 1, so CursorToggle flips the
        // cursor's Select and arms CursorToggled (C4Player.cpp:1322-1327).
        let mut engine = Engine::new();
        let [a, _, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).test_value();
        assert_eq!(engine.crew_cursor(1), Some(c));
        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).test_value();
        assert_eq!(
            engine.selected_crew(1),
            vec![c, a],
            "the new cursor's Select toggled ON"
        );
        assert_eq!(control_state(&engine, 1).cursor_toggled, 1);
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn regular_com_after_cursor_move_selects_single_by_cursor() {
        // UpdateSelectionToggleStatus (C4Player.cpp:1355-1365) runs on the
        // next regular com (C4Player::ObjectCom, :1378-1379): an untoggled
        // CursorSelection commits SelectSingleByCursor — only the cursor
        // stays selected.
        let mut engine = Engine::new();
        let [_, _, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).test_value();
        engine.player_in_com(1, COM_DOWN, 0).test_value();
        assert_eq!(
            engine.selected_crew(1),
            vec![c],
            "SelectSingleByCursor unselected the rest (C4Player.cpp:1308-1317)"
        );
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
    }

    #[test]
    fn cursor_toggle_double_selects_all_crew() {
        // COM_CursorToggle_D → SelectAllCrew (C4Player.cpp:1485,
        // 1341-1353): everyone Select, flags reset, Ding.
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).test_value();
        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).test_value();
        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![a, b, c]);
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
        assert_eq!(control_state(&engine, 1).cursor_toggled, 0);
        assert!(
            engine.pending_audio.iter().any(|command| matches!(
                command,
                crate::AudioCommand::PlaySound { name, .. } if name == "Ding"
            )),
            "SelectAllCrew plays Ding (C4Player.cpp:1352)"
        );
    }

    #[test]
    fn pure_cursor_toggle_flips_select_on_the_whole_crew() {
        // Without CursorSelection the toggle flips every non-disabled
        // crew's Select (C4Player.cpp:1329-1336) and re-adjusts the cursor
        // to the hirank Select (AdjustCursorCommand, :1235-1258).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).test_value();
        // a was selected -> off; b, c were unselected -> on.
        assert_eq!(engine.selected_crew(1), vec![c, b]);
        assert_eq!(
            engine.crew_cursor(1),
            Some(c),
            "AdjustCursorCommand moves the cursor to the first Select"
        );
        let _ = a;
    }

    #[test]
    fn cursor_com_script_override_consumes_the_cycling() {
        // C4Player::DirectCom's cursor half (C4Player.cpp:1457-1475): a
        // truthy ControlCursorRight on the cursor object consumes the com
        // before any cycling.
        let script = r#"
#strict
protected func ControlCursorRight() { return(1); }
"#;
        let mut engine = clonk_engine(script);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        let a = spawn_crew(&mut engine, "CLNK", 1);
        let b = engine.spawn_test_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk")),
        );

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).test_value();
        assert_eq!(
            engine.crew_cursor(1),
            Some(a),
            "the override kept the cursor in place"
        );
        let _ = b;
    }

    #[test]
    fn jump_and_run_down_keeps_grabbing_a_target_with_a_down_command() {
        // AutoStopDirectCom's DFA_PUSH/COM_Down branch retains the grab when
        // DrawCommandQuery exposes a JumpAndRun ControlDown command
        // (C4Object.cpp:3712-3721). The callback may legitimately be falsy;
        // its command metadata, not its return value, owns this gate.
        let vehicle = r#"
#strict
protected func ControlDown(pCaller)
{
  [$CtrlDown$|Method=JumpAndRun]
  DoDamage(1);
}
"#;
        let mut engine = clonk_engine("#strict\n");
        engine.register_test_script_definition("DRCK", "Derrick", vehicle);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine
            .players
            .get_mut(&1)
            .test_value()
            .control
            .control_style = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let derrick = engine.spawn_test_object(SpawnConfig::new("DRCK"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(derrick);

        engine.player_in_com(1, COM_DOWN, 0).test_value();

        assert_eq!(
            test_object(&engine, derrick).state.damage,
            1,
            "the target callback still runs first"
        );
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(snapshot.action.name, "Push");
        assert_eq!(snapshot.action.target, Some(derrick));
    }

    #[test]
    fn old_pushed_target_receives_autostop_control_after_clonk_fallback() {
        // AutoStopDirectCom uses the same 4.9.5 target-version boundary as
        // classic DFA_PUSH: old ControlLeft runs after AutoStopUpdateComDir
        // and cannot consume it (src/C4Object.cpp:3682-3738).
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = clonk_engine("#strict\n");
        let mut lorry = test_definition("LORY", "Lorry", vehicle);
        lorry.set_version([4, 9, 4, 9, 0]);
        engine.register_test_definition(lorry);
        engine.register_test_player(PlayerConfig::new(1, "Test"));
        engine
            .players
            .get_mut(&1)
            .test_value()
            .control
            .control_style = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let lorry = engine.spawn_test_object(SpawnConfig::new("LORY"));
        let crew_index = engine.test_object_index(crew);
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine.player_in_com(1, COM_LEFT, 0).test_value();

        assert_eq!(
            test_snapshot(&engine, crew).command_direction,
            CommandDirection::Left,
            "the old target's truthy late callback cannot consume auto-stop movement"
        );
        assert_eq!(
            test_snapshot(&engine, lorry).damage,
            1,
            "the old target still receives ControlLeft after movement"
        );
    }

    #[test]
    fn release_without_registered_press_is_dropped() {
        // C4Player::InCom (C4Player.cpp:1541-1548): a release only counts
        // when its press bit is set.
        let script = r#"
#strict
protected func ControlLeftReleased() { SetComDir(COMD_Right()); return(1); }
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);

        engine
            .player_in_com(1, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Stop,
            "unmatched releases never dispatch"
        );

        engine.player_in_com(1, COM_LEFT, 0).test_value();
        engine
            .player_in_com(1, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Right,
            "a registered release dispatches ControlLeftReleased"
        );
    }

    #[test]
    fn full_width_saved_last_com_narrows_only_when_direct_com_dispatches() {
        // C4Player::LastCom is int32_t, while DirectCom accepts uint8_t
        // (C4Player.h:121; C4Player.cpp:1215-1229,1490-1554). A compiler
        // word whose low byte equals the new com is still unequal as an int,
        // so native dispatches the old Single com before replacing it.
        let script = r#"
#strict
protected func ControlRightSingle() { DoDamage(1); return(1); }
protected func ControlRightDouble() { DoDamage(10); return(1); }
"#;
        let (mut engine, crew) = clonk_crew_fixture(script);
        engine.players.get_mut(&1).test_value().control.last_com = 0x102;

        engine.player_in_com(1, COM_RIGHT, 0).test_value();

        assert_eq!(test_snapshot(&engine, crew).damage, 1);
        assert_eq!(control_state(&engine, 1).last_com, i32::from(COM_RIGHT));
    }

    #[test]
    fn classic_release_does_not_stop_the_walk() {
        // In classic control a released direction key changes nothing: the
        // per-procedure switch has no release cases (C4Object.cpp:3406-3556).
        let (mut engine, crew) = clonk_crew_fixture("#strict\n");

        engine.player_in_com(1, COM_RIGHT, 0).test_value();
        engine
            .player_in_com(1, COM_RIGHT + COM_RELEASE_OFFSET, 0)
            .test_value();
        let snapshot = test_snapshot(&engine, crew);
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Right,
            "classic control keeps walking until COM_Down stops it"
        );
    }
}
