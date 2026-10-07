//! Copy-on-write vector storage for object state.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A `Vec` whose clones share storage until one side writes.
///
/// C++ script callbacks act on the live `C4Object`; the port hands each
/// callback a copy of its receiver's state (`Object::script_state_snapshot`)
/// and folds the outcome back. Collections that callbacks read far more often
/// than they write sit behind this, so that copy costs a reference count
/// instead of a deep clone of every element.
///
/// Reads go through `Deref<Target = Vec<T>>`; any mutable access detaches a
/// shared buffer first. `Debug` and serde output are those of the `Vec`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SharedVec<T>(Arc<Vec<T>>);

impl<T> SharedVec<T> {
    pub fn new() -> Self {
        Self(Arc::new(Vec::new()))
    }

    /// Shadows `Vec::is_empty` so serde's `skip_serializing_if` can name it.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_vec(self) -> Vec<T>
    where
        T: Clone,
    {
        Arc::unwrap_or_clone(self.0)
    }

    #[cfg(test)]
    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Default for SharedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Deref for SharedVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T: Clone> DerefMut for SharedVec<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: fmt::Debug> fmt::Debug for SharedVec<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.as_slice(), formatter)
    }
}

impl<T> From<Vec<T>> for SharedVec<T> {
    fn from(vec: Vec<T>) -> Self {
        Self(Arc::new(vec))
    }
}

impl<T: Clone> From<&[T]> for SharedVec<T> {
    fn from(slice: &[T]) -> Self {
        Self(Arc::new(slice.to_vec()))
    }
}

impl<T> FromIterator<T> for SharedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

impl<'a, T> IntoIterator for &'a SharedVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut SharedVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0).iter_mut()
    }
}

impl<T: Clone> IntoIterator for SharedVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<T: PartialEq> PartialEq<Vec<T>> for SharedVec<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.0.as_slice() == other.as_slice()
    }
}

impl<T: PartialEq> PartialEq<SharedVec<T>> for Vec<T> {
    fn eq(&self, other: &SharedVec<T>) -> bool {
        self.as_slice() == other.0.as_slice()
    }
}

impl<T: PartialEq> PartialEq<[T]> for SharedVec<T> {
    fn eq(&self, other: &[T]) -> bool {
        self.0.as_slice() == other
    }
}

impl<T: Serialize> Serialize for SharedVec<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_slice().serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for SharedVec<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_storage_until_written() {
        let original = SharedVec::from(vec![String::from("Fire"), String::from("Smoke")]);
        let mut copy = original.clone();
        assert!(copy.shares_storage_with(&original));

        copy.push(String::from("Glow"));
        assert!(!copy.shares_storage_with(&original));
        assert_eq!(original, vec![String::from("Fire"), String::from("Smoke")]);
        assert_eq!(copy.len(), 3);
    }

    #[test]
    fn a_sole_owner_writes_in_place() {
        let mut vec = SharedVec::from(vec![1, 2, 3]);
        let before = vec.as_ptr();
        vec[1] = 5;
        assert_eq!(vec.as_ptr(), before, "an unshared buffer is not copied");
        assert_eq!(vec, vec![1, 5, 3]);
    }

    /// Debug and serde output are part of snapshot and savegame text, so the
    /// wrapper must not show up in either.
    #[test]
    fn formats_and_serialises_as_the_plain_vec() {
        let shared = SharedVec::from(vec![1, 2]);
        assert_eq!(format!("{shared:?}"), format!("{:?}", vec![1, 2]));
        assert_eq!(format!("{shared:#?}"), format!("{:#?}", vec![1, 2]));
        let json = serde_json::to_string(&shared).expect("serialises");
        assert_eq!(json, "[1,2]");
        assert_eq!(
            serde_json::from_str::<SharedVec<i32>>(&json).expect("round trips"),
            shared
        );
    }
}
