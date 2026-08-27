//! The internal object-menu model shared by the activate, contents and
//! base menus: `C4Menu` refill tokens, the removal-safe cursor, and the
//! picture-group concatenation rules.

use super::*;

pub(in crate::direct_com) fn internal_object_menu_picture_groups<S: InternalObjectMenuSource>(
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

pub(in crate::direct_com) fn internal_live_contents_definition_count<
    S: InternalObjectMenuSource,
>(
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

pub(in crate::direct_com) fn internal_live_contents_count<S: InternalObjectMenuSource>(
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

pub(in crate::direct_com) fn internal_refilled_object_menu_selection(
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

pub(in crate::direct_com) fn internal_object_menu_selected_definition(
    menu: &crate::ObjectMenuState,
) -> Option<String> {
    usize::try_from(menu.selection)
        .ok()
        .and_then(|selection| menu.items.get(selection))
        // C4ObjectMenu::checkIDSelection explicitly skips C4ID_None.
        .filter(|item| item.item_id != "NONE")
        .map(|item| item.item_id.clone())
}

pub(in crate::direct_com) fn activate_menu_state(
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

pub(in crate::direct_com) fn next_internal_object_menu_refill_token() -> u64 {
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
pub(in crate::direct_com) struct InternalObjectMenuLink {
    object: ObjectId,
    generation: u64,
}

pub(in crate::direct_com) struct InternalObjectMenuMutationTracker {
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

pub(in crate::direct_com) struct InternalObjectMenuMutationGuard {
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

pub(in crate::direct_com) fn internal_object_menu_has_enclosing_refill(
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

pub(in crate::direct_com) fn internal_object_menu_removed_successor(
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
pub(in crate::direct_com) enum InternalObjectMenuIteratorPosition {
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
pub(in crate::direct_com) struct InternalObjectMenuSafeCursor {
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

pub(in crate::direct_com) fn internal_object_menu_links<S: InternalObjectMenuSource>(
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
pub(in crate::direct_com) fn internal_object_menu_iterator_next<S: InternalObjectMenuSource>(
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

pub(in crate::direct_com) struct EngineInternalObjectMenuSource<'a>(
    pub(in crate::direct_com) &'a mut Engine,
);

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
pub(in crate::direct_com) enum DigDoublePhysicalBacking {
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
pub(in crate::direct_com) const COM_DOWN_D: u8 = COM_DOWN | COM_DOUBLE;
pub(in crate::direct_com) const COM_DIG_S: u8 = COM_DIG | COM_SINGLE;
pub(in crate::direct_com) const COM_DIG_D: u8 = COM_DIG | COM_DOUBLE;
pub(in crate::direct_com) const COM_THROW_D: u8 = COM_THROW | COM_DOUBLE;

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
