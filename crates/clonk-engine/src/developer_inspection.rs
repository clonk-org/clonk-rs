//! The console's object-inspection read model.
//!
//! The developer windows need C++'s *native* ordering, which is not the order
//! any existing snapshot exposes:
//!
//! - `C4ObjectListDlg`'s tree walks `Game.Objects` **First → Next** and skips
//!   every contained object at the top level (`C4ObjectListDlg.cpp:100-101`,
//!   repeated at `:557-560`); each row's children are that object's `Contents`
//!   list, where every entry is contained by definition and nothing is skipped.
//! - The port's [`SimulationSnapshot::render_order`] is the *draw* direction,
//!   `Last → Prev` (`C4ObjectList.cpp:390-395`), so the tree order is its
//!   reverse. `objects` itself is ID-sorted and carries no list order at all.
//! - `C4PropertyDlg::Update` (`C4PropertyDlg.cpp:196-201`) reads the selected
//!   object's contents as `Contents.GetNameList(Game.Defs)`, which is
//!   first-seen definition order with counts — not the raw contents list.
//!
//! This module is a pure projection: it computes order, and leaves text and
//! layout to [`crate::developer_property_text`].

use crate::{DefinitionId, ObjectId, ObjectStatus, SimulationSnapshot};

/// One row of the console object tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectionNode {
    pub id: ObjectId,
    /// The object's `Contents` in list order. Empty for a leaf.
    pub contents: Vec<InspectionNode>,
}

/// The per-object facts the tree and name list are built from, so callers may
/// supply them from a snapshot, from live state, or from a test fixture.
pub trait InspectionSource {
    /// `C4Object::Contained`.
    fn container_of(&self, id: ObjectId) -> Option<ObjectId>;
    /// `C4Object::Contents` in `C4ObjectList` order.
    fn contents_of(&self, id: ObjectId) -> Vec<ObjectId>;
    /// `C4Object::Def->id`.
    fn definition_of(&self, id: ObjectId) -> Option<DefinitionId>;
    /// `C4Object::Status`, which C++ tests for truth — deleted objects are
    /// excluded from counts, *inactive* ones are not.
    fn deleted(&self, id: ObjectId) -> bool;
}

impl InspectionSource for SimulationSnapshot {
    fn container_of(&self, id: ObjectId) -> Option<ObjectId> {
        self.object(id).and_then(|object| object.container)
    }

    fn contents_of(&self, id: ObjectId) -> Vec<ObjectId> {
        self.object(id)
            .map(|object| object.contents.clone())
            .unwrap_or_default()
    }

    fn definition_of(&self, id: ObjectId) -> Option<DefinitionId> {
        self.object(id).map(|object| object.definition_id.clone())
    }

    fn deleted(&self, id: ObjectId) -> bool {
        self.object(id)
            .is_none_or(|object| object.status == ObjectStatus::Deleted)
    }
}

/// `Game.Objects` in C++'s First → Next order.
///
/// `render_order` is the draw direction (`Last → Prev`), so the list order is
/// its reverse.
pub fn master_list_order(render_order: &[ObjectId]) -> Vec<ObjectId> {
    render_order.iter().rev().copied().collect()
}

/// The console object tree (`C4ObjectListDlg.cpp:100-101,557-560`).
///
/// Top-level rows are `Game.Objects` in list order with every *contained*
/// object skipped; each row's children are its own `Contents` list, recursively.
pub fn object_tree(
    render_order: &[ObjectId],
    source: &impl InspectionSource,
) -> Vec<InspectionNode> {
    master_list_order(render_order)
        .into_iter()
        // "Skip Contained Objects in the main list" — they appear under their
        // container instead.
        .filter(|id| source.container_of(*id).is_none())
        .map(|id| node(id, source))
        .collect()
}

fn node(id: ObjectId, source: &impl InspectionSource) -> InspectionNode {
    InspectionNode {
        id,
        contents: source
            .contents_of(id)
            .into_iter()
            .map(|contained| node(contained, source))
            .collect(),
    }
}

/// `MaxTempListID` (`C4ObjectList.cpp:55`).
pub const MAX_TEMP_LIST_ID: usize = 500;

/// The distinct definitions of an object list in first-seen order, with counts
/// (`C4ObjectList::GetListID` / `ObjectCount`, `C4ObjectList.cpp:59-83,536-547`).
///
/// Deleted objects are excluded; inactive ones are not, because C++ tests
/// `clnk->Obj->Status` rather than comparing against `C4OS_NORMAL`. The list is
/// capped at [`MAX_TEMP_LIST_ID`] distinct definitions, exactly as C++'s fixed
/// `TempListID` buffer is.
pub fn definition_counts(
    ids: &[ObjectId],
    source: &impl InspectionSource,
) -> Vec<(DefinitionId, u32)> {
    let mut counts: Vec<(DefinitionId, u32)> = Vec::new();
    for definition in ids
        .iter()
        .filter(|id| !source.deleted(**id))
        .filter_map(|id| source.definition_of(*id))
    {
        match counts.iter().position(|(id, _)| *id == definition) {
            Some(slot) => counts[slot].1 += 1,
            // The temporary id buffer is a fixed 500 slots; C++ silently drops
            // any further definition rather than growing.
            None if counts.len() < MAX_TEMP_LIST_ID => counts.push((definition, 1)),
            None => {}
        }
    }
    counts
}

/// `C4ObjectList::GetNameList` (`C4ObjectList.cpp:560-574`).
///
/// `name_of` resolves a definition through `C4DefList::ID2Def`; returning
/// `None` mirrors an unknown id, which C++ skips. Note the separator is keyed
/// on the *index* rather than on what was emitted, so a skipped leading entry
/// still leaves the `", "` in front of the first name — a C++ quirk this
/// preserves.
pub fn name_list(
    ids: &[ObjectId],
    source: &impl InspectionSource,
    name_of: impl Fn(&str) -> Option<String>,
) -> String {
    definition_counts(ids, source)
        .into_iter()
        .enumerate()
        .filter_map(|(index, (id, count))| name_of(&id).map(|name| (index, count, name)))
        .fold(String::new(), |mut result, (index, count, name)| {
            if index > 0 {
                result.push_str(", ");
            }
            result.push_str(&format!("{count}x {name}"));
            result
        })
}

/// The property panel's effect lines (`C4PropertyDlg.cpp:236-247`).
///
/// C++ walks `cobj->pEffects` through `pNext` and writes
/// `" {name}: Interval {interval}"` for each. The port's
/// `ObjectState::effects` is that list in the same order, so this is a
/// straight projection — no sorting.
pub fn effect_lines(effects: &[crate::effect::EffectState]) -> Vec<String> {
    effects
        .iter()
        .map(|effect| format!(" {}: Interval {}", effect.name, effect.interval))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fixture {
        container: HashMap<ObjectId, ObjectId>,
        contents: HashMap<ObjectId, Vec<ObjectId>>,
        definition: HashMap<ObjectId, DefinitionId>,
        deleted: Vec<ObjectId>,
    }

    impl Fixture {
        fn contained(mut self, id: u64, container: u64) -> Self {
            self.container.insert(ObjectId(id), ObjectId(container));
            self.contents
                .entry(ObjectId(container))
                .or_default()
                .push(ObjectId(id));
            self
        }

        fn definition(mut self, id: u64, definition: &str) -> Self {
            self.definition.insert(ObjectId(id), definition.to_owned());
            self
        }

        fn deleted(mut self, id: u64) -> Self {
            self.deleted.push(ObjectId(id));
            self
        }
    }

    impl InspectionSource for Fixture {
        fn container_of(&self, id: ObjectId) -> Option<ObjectId> {
            self.container.get(&id).copied()
        }

        fn contents_of(&self, id: ObjectId) -> Vec<ObjectId> {
            self.contents.get(&id).cloned().unwrap_or_default()
        }

        fn definition_of(&self, id: ObjectId) -> Option<DefinitionId> {
            self.definition.get(&id).cloned()
        }

        fn deleted(&self, id: ObjectId) -> bool {
            self.deleted.contains(&id)
        }
    }

    fn ids(raw: [u64; 5]) -> Vec<ObjectId> {
        raw.into_iter().map(ObjectId).collect()
    }

    // C4ObjectListDlg.cpp:100-101,557-560 and C4ObjectList.cpp:390-395 — the
    // tree is the draw order reversed, with contained objects shown only under
    // their container, and each row's children in the container's own order.
    #[test]
    fn developer_object_inspection_preserves_master_contents_local_and_effect_order() {
        // 1 holds 3 then 2; 3 in turn holds 5. 4 is free-standing.
        let world = Fixture::default()
            .contained(3, 1)
            .contained(2, 1)
            .contained(5, 3);

        // Draw walks Last -> Prev, so the list order is the reverse.
        assert_eq!(
            master_list_order(&ids([5, 4, 3, 2, 1])),
            ids([1, 2, 3, 4, 5])
        );

        let tree = object_tree(&ids([5, 4, 3, 2, 1]), &world);
        assert_eq!(
            tree,
            vec![
                InspectionNode {
                    id: ObjectId(1),
                    contents: vec![
                        // The container's own list order, not the master order:
                        // 3 was added before 2.
                        InspectionNode {
                            id: ObjectId(3),
                            contents: vec![InspectionNode {
                                id: ObjectId(5),
                                contents: Vec::new(),
                            }],
                        },
                        InspectionNode {
                            id: ObjectId(2),
                            contents: Vec::new(),
                        },
                    ],
                },
                InspectionNode {
                    id: ObjectId(4),
                    contents: Vec::new(),
                },
            ],
            "contained objects appear only under their container"
        );

        // An empty world has no rows at all.
        assert!(object_tree(&[], &world).is_empty());

        // C4PropertyDlg.cpp:236-247 — effects keep their list order, which is
        // priority order, not the order they were added.
        use crate::effect::EffectState;
        let effects = vec![
            EffectState {
                interval: 1,
                ..EffectState::new("Fire")
            },
            EffectState {
                interval: 0,
                ..EffectState::new("Smoke")
            },
        ];
        assert_eq!(
            effect_lines(&effects),
            vec![
                " Fire: Interval 1".to_owned(),
                " Smoke: Interval 0".to_owned()
            ]
        );
        assert!(effect_lines(&[]).is_empty());
    }

    // C4ObjectList.cpp:59-83,536-574 — first-seen definition order with counts,
    // deleted objects excluded, unknown definitions skipped.
    #[test]
    fn developer_object_inspection_exposes_data_strings() {
        let contents = Fixture::default()
            .definition(1, "ROCK")
            .definition(2, "WOOD")
            .definition(3, "ROCK")
            .definition(4, "GOLD")
            .definition(5, "WOOD")
            .deleted(4);

        // First seen wins the slot; later duplicates only bump the count. The
        // deleted GOLD never appears.
        assert_eq!(
            definition_counts(&ids([1, 2, 3, 4, 5]), &contents),
            vec![("ROCK".to_owned(), 2), ("WOOD".to_owned(), 2)]
        );

        let names = |id: &str| match id {
            "ROCK" => Some("Rock".to_owned()),
            "WOOD" => Some("Wood".to_owned()),
            _ => None,
        };
        assert_eq!(
            name_list(&ids([1, 2, 3, 4, 5]), &contents, names),
            "2x Rock, 2x Wood"
        );

        // An unknown definition is skipped, but the separator is keyed on the
        // list index, so dropping the *first* entry still leaves its comma.
        let unknown_first = Fixture::default()
            .definition(1, "XXXX")
            .definition(2, "ROCK")
            .definition(3, "ROCK")
            .definition(4, "ROCK")
            .definition(5, "ROCK");
        assert_eq!(
            name_list(&ids([1, 2, 3, 4, 5]), &unknown_first, names),
            ", 4x Rock",
            "C++ keys the separator on cpos, not on what it emitted"
        );

        // Nothing to list is the empty string, not a stray separator.
        assert_eq!(name_list(&[], &contents, names), "");

        // C4Value.cpp GetDataString — the exact strings the panel prints for
        // local and effect values. No object formatter is installed here, so an
        // object prints as its bare number, as C++ does when it cannot find it.
        use clonk_script::{data_string, Value, ValueMap};
        assert_eq!(data_string(&Value::Nil), "nil");
        assert_eq!(data_string(&Value::Int(-7)), "-7");
        assert_eq!(data_string(&Value::Bool(true)), "true");
        assert_eq!(data_string(&Value::String("hi".into())), "\"hi\"");
        assert_eq!(data_string(&Value::Object(42)), "42");
        assert_eq!(
            data_string(&Value::Array(vec![Value::Int(1), Value::Nil])),
            "[1, nil]"
        );
        assert_eq!(data_string(&Value::Proplist(ValueMap::new())), "{}");
        let mut map = ValueMap::new();
        map.insert("k".to_owned(), Value::Int(2));
        assert_eq!(data_string(&Value::Proplist(map)), "{ \"k\" = 2 }");
    }
}
