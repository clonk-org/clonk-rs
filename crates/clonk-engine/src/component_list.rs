//! `C4IDList`: an ordered ID/count list that may repeat an ID.
//!
//! C++ stores `std::vector<Entry>` (`C4IDList.h`), so position is meaningful
//! and the same ID may appear more than once with independent counts. The
//! shipped Bazooka `DefCore` does exactly that — `Components=METL=2;KLAS=1;
//! ENAP=1;ENAP=1` — and `C4Object::ComponentConGain`/`Cutoff` index the
//! definition's list **by position**, not by ID (`C4Object.cpp:510-526`), so a
//! representation that collapses repeats reads the wrong definition entry.
//!
//! Lookup by ID resolves to the **first** matching entry, which is what
//! `findId` does for `GetIDCount`/`SetIDCount` (`C4IDList.cpp:60-66,76-83`).

use std::collections::HashMap;
use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::DefinitionId;

/// An ordered ID/count list mirroring `C4IDList`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentList {
    entries: Vec<(DefinitionId, i32)>,
}

impl ComponentList {
    pub fn new() -> Self {
        Self::default()
    }

    /// `GetNumberOfIDs` (`C4IDList.cpp:33-36`): the entry count, which counts a
    /// repeated ID once per occurrence.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `GetIDCount` (`C4IDList.cpp:76-83`): the first entry with this ID.
    pub fn get(&self, id: &str) -> Option<i32> {
        self.entries
            .iter()
            .find(|(entry, _)| entry.as_str() == id)
            .map(|(_, count)| *count)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// `GetCount(index)` (`C4IDList.cpp:47-51`): positional, which is how the
    /// construction-progress paths read a definition's list.
    pub fn count_at(&self, index: usize) -> Option<i32> {
        self.entries.get(index).map(|(_, count)| *count)
    }

    pub fn id_at(&self, index: usize) -> Option<&DefinitionId> {
        self.entries.get(index).map(|(id, _)| id)
    }

    /// `SetCount(index, count)` (`C4IDList.cpp:53-58`).
    pub fn set_count_at(&mut self, index: usize, count: i32) -> bool {
        match self.entries.get_mut(index) {
            Some((_, existing)) => {
                *existing = count;
                true
            }
            None => false,
        }
    }

    /// `SetIDCount` (`C4IDList.cpp:85-`): updates the **first** entry with this
    /// ID, appending when absent.
    pub fn set(&mut self, id: DefinitionId, count: i32) {
        match self
            .entries
            .iter_mut()
            .find(|(entry, _)| *entry == id)
            .map(|(_, existing)| existing)
        {
            Some(existing) => *existing = count,
            None => self.entries.push((id, count)),
        }
    }

    /// Appends without merging, so a caller replaying C++'s parse order keeps
    /// its repeats.
    pub fn push(&mut self, id: DefinitionId, count: i32) {
        self.entries.push((id, count));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DefinitionId, i32)> {
        self.entries.iter().map(|(id, count)| (id, *count))
    }

    pub fn ids(&self) -> impl Iterator<Item = &DefinitionId> {
        self.entries.iter().map(|(id, _)| id)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&DefinitionId, i32) -> bool) {
        self.entries.retain(|(id, count)| keep(id, *count));
    }
}

impl Serialize for ComponentList {
    /// Emitted as a sequence, because a map cannot hold a repeated ID.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.entries.len()))?;
        for (id, count) in &self.entries {
            sequence.serialize_element(&(id, count))?;
        }
        sequence.end()
    }
}

impl<'de> Deserialize<'de> for ComponentList {
    /// Accepts **either** shape. States written before this type existed hold a
    /// map (`{"WOOD": 4}`); states written since hold a sequence
    /// (`[["WOOD", 4]]`). A savegame or engine snapshot recorded earlier has to
    /// keep loading, so the map arm is a compatibility path rather than dead
    /// code — it is what every already-written save on disk uses.
    ///
    /// The map arm sorts by ID: map iteration order is not deterministic, and a
    /// state recorded in the old shape carries no order to recover.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EitherShape;

        impl<'de> Visitor<'de> for EitherShape {
            type Value = ComponentList;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a component sequence or a legacy id->count map")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::with_capacity(access.size_hint().unwrap_or_default());
                while let Some(entry) = access.next_element::<(DefinitionId, i32)>()? {
                    entries.push(entry);
                }
                Ok(ComponentList { entries })
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::with_capacity(access.size_hint().unwrap_or_default());
                while let Some(entry) = access.next_entry::<DefinitionId, i32>()? {
                    entries.push(entry);
                }
                entries.sort_by(|(left, _), (right, _)| left.cmp(right));
                Ok(ComponentList { entries })
            }
        }

        deserializer.deserialize_any(EitherShape)
    }
}

impl FromIterator<(DefinitionId, i32)> for ComponentList {
    fn from_iter<T: IntoIterator<Item = (DefinitionId, i32)>>(iter: T) -> Self {
        Self {
            entries: iter.into_iter().collect(),
        }
    }
}

impl From<HashMap<DefinitionId, i32>> for ComponentList {
    /// Recovers a list from an old map-shaped state. Map iteration is
    /// unordered, so the entries are sorted by ID to keep this deterministic;
    /// callers that know C++'s order build the list directly instead.
    fn from(map: HashMap<DefinitionId, i32>) -> Self {
        let mut entries = map.into_iter().collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        Self { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> DefinitionId {
        DefinitionId::from(name)
    }

    /// The shipped Bazooka `DefCore` repeats ENAP, and `GetNumberOfIDs` counts
    /// both (`C4IDList.cpp:33-36`), where a map keyed by ID reports three.
    #[test]
    fn repeated_ids_stay_separate_entries() {
        let list = ComponentList::from_iter([
            (id("METL"), 2),
            (id("KLAS"), 1),
            (id("ENAP"), 1),
            (id("ENAP"), 1),
        ]);
        assert_eq!(list.len(), 4);
        assert_eq!(list.id_at(2), Some(&id("ENAP")));
        assert_eq!(list.id_at(3), Some(&id("ENAP")));
    }

    /// Unequal counts are the case a map cannot represent at all: it keeps one
    /// of the two. Positional reads see both, and ID lookup takes the first,
    /// as `findId` does (`C4IDList.cpp:60-66,76-83`).
    #[test]
    fn repeated_ids_keep_independent_counts() {
        let mut list = ComponentList::new();
        list.push(id("ROCK"), 3);
        list.push(id("ROCK"), 7);

        assert_eq!(list.len(), 2);
        assert_eq!(list.count_at(0), Some(3));
        assert_eq!(list.count_at(1), Some(7));
        assert_eq!(list.get("ROCK"), Some(3), "ID lookup takes the first entry");

        // SetIDCount also addresses the first, leaving the second alone.
        list.set(id("ROCK"), 9);
        assert_eq!(list.count_at(0), Some(9));
        assert_eq!(list.count_at(1), Some(7));

        // Positional writes reach the second.
        assert!(list.set_count_at(1, 11));
        assert_eq!(list.count_at(1), Some(11));
        assert!(!list.set_count_at(2, 1), "out of range is a no-op");
    }

    /// Round-trips as a sequence, which is the only shape that can hold the
    /// repeat the Bazooka DefCore ships.
    #[test]
    fn serialises_as_a_sequence_and_round_trips_repeats() {
        let list = ComponentList::from_iter([(id("ENAP"), 1), (id("ENAP"), 2)]);
        let json = serde_json::to_string(&list).expect("serialises");
        assert_eq!(json, r#"[["ENAP",1],["ENAP",2]]"#);
        assert_eq!(
            serde_json::from_str::<ComponentList>(&json).expect("round trips"),
            list
        );
    }

    /// A state written before this type existed holds a map; it still loads.
    #[test]
    fn deserialises_the_legacy_map_shape() {
        let list = serde_json::from_str::<ComponentList>(r#"{"WOOD":4,"METL":2}"#)
            .expect("legacy map loads");
        assert_eq!(list.len(), 2);
        assert_eq!(list.get("WOOD"), Some(4));
        assert_eq!(list.get("METL"), Some(2));
        assert_eq!(
            list.ids().cloned().collect::<Vec<_>>(),
            vec![id("METL"), id("WOOD")],
            "map order is not deterministic, so the recovery sorts by ID"
        );
    }

    #[test]
    fn insertion_order_is_preserved_verbatim() {
        let list = ComponentList::from_iter([(id("ZZZZ"), 1), (id("AAAA"), 2)]);
        assert_eq!(
            list.ids().cloned().collect::<Vec<_>>(),
            vec![id("ZZZZ"), id("AAAA")],
            "the list is not sorted"
        );
    }
}
