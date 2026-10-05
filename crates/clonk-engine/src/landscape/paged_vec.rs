//! Flat reads and writes while uniquely owned; bounded copy-on-write pages
//! when a callback or snapshot shares the array. Contiguous views are built
//! only for consumers that explicitly request the complete array.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::sync::{Arc, OnceLock};

const BLOCK_PAGES: usize = 64;
type Block<T> = [Option<Arc<Vec<T>>>; BLOCK_PAGES];
type Branch<T> = [Option<Arc<Block<T>>>; BLOCK_PAGES];

#[derive(Debug, Clone)]
pub(super) struct PagedVec<T, const PAGE: usize = 32> {
    base: Arc<Vec<T>>,
    blocks: Arc<Vec<Option<Arc<Branch<T>>>>>,
    // A bounded bloom filter skips all overlay pointer chasing for reads
    // outside changed blocks. Collisions merely fall through to the tree.
    patched_blocks: u64,
    contiguous: OnceLock<Arc<Vec<T>>>,
}

impl<T, const PAGE: usize> From<Vec<T>> for PagedVec<T, PAGE> {
    fn from(base: Vec<T>) -> Self {
        let count = base
            .len()
            .div_ceil(PAGE)
            .div_ceil(BLOCK_PAGES * BLOCK_PAGES);
        Self {
            base: Arc::new(base),
            blocks: Arc::new((0..count).map(|_| None).collect()),
            patched_blocks: 0,
            contiguous: OnceLock::new(),
        }
    }
}

impl<T, const PAGE: usize> Default for PagedVec<T, PAGE> {
    fn default() -> Self {
        Vec::new().into()
    }
}

impl<T, const PAGE: usize> PagedVec<T, PAGE> {
    #[inline]
    pub(super) fn len(&self) -> usize {
        self.base.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub(super) fn is_shared(&self) -> bool {
        if self.patched_blocks != 0 {
            Arc::strong_count(&self.blocks) > 1
        } else {
            Arc::strong_count(&self.base) > 1
        }
    }

    pub(super) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.base, &other.base) && Arc::ptr_eq(&self.blocks, &other.blocks)
    }

    #[inline]
    pub(super) fn get(&self, index: usize) -> Option<&T> {
        if self.patched_blocks == 0 {
            return self.base.get(index);
        }
        let original = self.base.get(index)?;
        let page = index / PAGE;
        if self.patched_blocks & (1 << ((page / BLOCK_PAGES) % 64)) != 0 {
            if let Some(data) = self.blocks[page / (BLOCK_PAGES * BLOCK_PAGES)]
                .as_ref()
                .and_then(|branch| branch[(page / BLOCK_PAGES) % BLOCK_PAGES].as_ref())
                .and_then(|block| block[page % BLOCK_PAGES].as_ref())
            {
                return data.get(index % PAGE);
            }
        }
        Some(original)
    }

    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = &T> + DoubleEndedIterator {
        (0..self.len()).map(|index| &self[index])
    }
}

impl<T: Clone, const PAGE: usize> PagedVec<T, PAGE> {
    pub(super) fn as_vec(&self) -> &Vec<T> {
        if self.patched_blocks == 0 {
            return &self.base;
        }
        self.contiguous.get_or_init(|| {
            let mut data = self.base.as_ref().clone();
            self.apply_pages(&mut data);
            Arc::new(data)
        })
    }

    pub(super) fn as_slice(&self) -> &[T] {
        self.as_vec()
    }

    fn apply_pages(&self, data: &mut [T]) {
        for (branch_index, branch) in self.blocks.iter().enumerate() {
            let Some(branch) = branch else { continue };
            for (block_index, block) in branch.iter().enumerate() {
                let Some(block) = block else { continue };
                for (page_index, page) in block.iter().enumerate() {
                    if let Some(page) = page {
                        let start = ((branch_index * BLOCK_PAGES + block_index) * BLOCK_PAGES
                            + page_index)
                            * PAGE;
                        data[start..start + page.len()].clone_from_slice(page);
                    }
                }
            }
        }
    }

    pub(super) fn make_contiguous_mut(&mut self) -> &mut Vec<T> {
        if self.patched_blocks != 0 {
            if Arc::strong_count(&self.base) == 1 {
                // Once the last preview is dropped, retain the original flat
                // allocation and fold just its changed pages back into it.
                let mut base = std::mem::take(&mut self.base);
                self.apply_pages(Arc::get_mut(&mut base).expect("uniquely owned base"));
                self.base = base;
            } else {
                self.base = self.contiguous.take().unwrap_or_else(|| {
                    let mut data = self.base.as_ref().clone();
                    self.apply_pages(&mut data);
                    Arc::new(data)
                });
            }
            self.blocks = Arc::new((0..self.blocks.len()).map(|_| None).collect());
            self.patched_blocks = 0;
        }
        self.contiguous.take();
        Arc::make_mut(&mut self.base)
    }

    pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            return None;
        }
        if let Some(flat) = self.contiguous.take() {
            // A renderer/save consumer already paid for this exact flat
            // view. Reuse it as the baseline instead of discarding it and
            // accumulating overlays against an older retained snapshot.
            self.base = flat;
            self.blocks = Arc::new((0..self.blocks.len()).map(|_| None).collect());
            self.patched_blocks = 0;
        }
        if Arc::strong_count(&self.base) == 1 {
            return self.make_contiguous_mut().get_mut(index);
        }
        let page = index / PAGE;
        let start = page * PAGE;
        let end = (start + PAGE).min(self.len());
        let branch = Arc::make_mut(&mut self.blocks)[page / (BLOCK_PAGES * BLOCK_PAGES)]
            .get_or_insert_with(|| Arc::new(std::array::from_fn(|_| None)));
        let block = Arc::make_mut(branch)[(page / BLOCK_PAGES) % BLOCK_PAGES]
            .get_or_insert_with(|| Arc::new(std::array::from_fn(|_| None)));
        let data = Arc::make_mut(block)[page % BLOCK_PAGES]
            .get_or_insert_with(|| Arc::new(self.base[start..end].to_vec()));
        self.patched_blocks |= 1 << ((page / BLOCK_PAGES) % 64);
        Arc::make_mut(data).get_mut(index % PAGE)
    }

    pub(super) fn resize(&mut self, length: usize, value: T) {
        if length != self.len() {
            self.make_contiguous_mut().resize(length, value);
            self.blocks = Arc::new(
                (0..length.div_ceil(PAGE).div_ceil(BLOCK_PAGES * BLOCK_PAGES))
                    .map(|_| None)
                    .collect(),
            );
        }
    }
}

impl<T, const PAGE: usize> Index<usize> for PagedVec<T, PAGE> {
    type Output = T;
    #[inline]
    fn index(&self, index: usize) -> &T {
        self.get(index).expect("array index in bounds")
    }
}

impl<T: Clone, const PAGE: usize> IndexMut<usize> for PagedVec<T, PAGE> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.get_mut(index).expect("array index in bounds")
    }
}

impl<T: Clone, const PAGE: usize> Deref for PagedVec<T, PAGE> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        self.as_vec()
    }
}

impl<T: Clone, const PAGE: usize> DerefMut for PagedVec<T, PAGE> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        self.make_contiguous_mut()
    }
}

impl<T: PartialEq, const PAGE: usize> PartialEq for PagedVec<T, PAGE> {
    fn eq(&self, other: &Self) -> bool {
        self.shares_storage(other) || (self.len() == other.len() && self.iter().eq(other.iter()))
    }
}

impl<T: Eq, const PAGE: usize> Eq for PagedVec<T, PAGE> {}

impl<T: Clone + Serialize, const PAGE: usize> Serialize for PagedVec<T, PAGE> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_vec().serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>, const PAGE: usize> Deserialize<'de> for PagedVec<T, PAGE> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::deserialize(deserializer).map(Self::from)
    }
}

impl<'a, T: Clone, const PAGE: usize> IntoIterator for &'a PagedVec<T, PAGE> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a, T: Clone, const PAGE: usize> IntoIterator for &'a mut PagedVec<T, PAGE> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.make_contiguous_mut().iter_mut()
    }
}

impl<T: Clone, const PAGE: usize> Index<std::ops::Range<usize>> for PagedVec<T, PAGE> {
    type Output = [T];
    fn index(&self, index: std::ops::Range<usize>) -> &[T] {
        &self.as_slice()[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_materialized_view_becomes_the_next_write_baseline() {
        let original = PagedVec::<u8, 32>::from(vec![7; 4097]);
        let mut branch = original.clone();
        branch[1] = 8;
        let flat = branch.as_slice().as_ptr();
        let snapshot = branch.clone();
        branch[64] = 9;
        assert_eq!(branch.base.as_ptr(), flat);
        assert_eq!(branch[1], 8);
        assert_eq!(branch[64], 9);
        assert_eq!(snapshot.as_slice()[64], 7);
        assert_eq!(original[1], 7);
    }

    #[test]
    fn paged_branches_preserve_views_through_writes_and_bulk_resize() {
        let original = PagedVec::<u8, 32>::from(vec![7; 131073]);
        let mut branch = original.clone();
        for slot in [0, 31, 32, 2047, 2048, 4096, 131072] {
            branch[slot] = 8;
            assert_eq!(branch[slot], 8);
            assert_eq!(original[slot], 7);
        }
        // The first and last writes share a bloom bit across tree branches;
        // untouched slots in those blocks must still read their base bytes.
        for slot in [33, 4095, 131071] {
            assert_eq!(branch[slot], 7);
        }
        let snapshot = branch.clone();
        assert_eq!(branch.as_slice(), snapshot.as_slice());
        branch[2048] = 9;
        assert_eq!(snapshot.as_slice()[2048], 8);
        assert_eq!(branch.as_slice()[2048], 9);
        drop(original);
        drop(snapshot);
        // The last branch folds only changed pages into its uniquely owned
        // base, then ordinary writes reuse that flat allocation.
        branch[1] = 10;
        assert_eq!(branch.patched_blocks, 0);
        assert_eq!(branch[2048], 9);
        branch.resize(137072, 11);
        let before = branch.clone();
        branch[137071] = 12;
        assert_eq!(before[137071], 11);
        assert_eq!(branch[137071], 12);
        branch.resize(6000, 99);
        assert_eq!(branch.len(), 6000);
        assert_eq!(branch[5999], 7);
        assert_eq!(before.len(), 137072);
        let json = serde_json::to_string(&branch).unwrap();
        let restored: PagedVec<u8, 32> = serde_json::from_str(&json).unwrap();
        assert_eq!(branch, restored);
    }
}
