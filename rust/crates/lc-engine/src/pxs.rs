//! Port of the C4PXS pixel-sprite system storage (src/C4PXS.{h,cpp}).
//!
//! PXS are sync-relevant: they carry `C4Fixed` state, consume the synced
//! `Random()` stream (wind drift, casts), and react with the landscape. The
//! chunk/slot storage is part of observable behavior — `New()` fills the
//! lowest free slot and `Execute()` iterates chunks then slots in order, so
//! slot reuse determines deterministic execution order.

use crate::math::{itofix, C4Fixed};
use crate::rng::LcgRng;
use crate::MaterialId;
use serde::{Deserialize, Serialize};

/// `PXSChunkSize` / `PXSMaxChunk` (C4PXS.h:40).
pub const PXS_CHUNK_SIZE: usize = 500;
pub const PXS_MAX_CHUNK: usize = 20;

/// One pixel sprite (C4PXS.h:25-38). `Mat == MNone` (a free slot) is modeled
/// as `None` in the chunk arrays instead of a sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pxs {
    pub mat: MaterialId,
    pub x: C4Fixed,
    pub y: C4Fixed,
    pub xdir: C4Fixed,
    pub ydir: C4Fixed,
}

/// Mirror of `C4PXSSystem` (C4PXS.h:42-70) minus drawing.
#[derive(Debug, Clone, Default)]
pub struct PxsSystem {
    chunks: Vec<Option<Vec<Option<Pxs>>>>,
    chunk_counts: Vec<usize>,
}

impl PxsSystem {
    fn ensure_layout(&mut self) {
        if self.chunks.len() != PXS_MAX_CHUNK {
            self.chunks.resize_with(PXS_MAX_CHUNK, || None);
            self.chunk_counts.resize(PXS_MAX_CHUNK, 0);
        }
    }

    /// `C4PXSSystem::New` (C4PXS.cpp:175-199): scan chunks in order,
    /// allocating missing ones; within a non-full chunk take the first free
    /// slot. Followed by `Create`'s field init (C4PXS.cpp:201-210).
    pub fn create(
        &mut self,
        mat: MaterialId,
        x: C4Fixed,
        y: C4Fixed,
        xdir: C4Fixed,
        ydir: C4Fixed,
    ) -> bool {
        self.ensure_layout();
        for chunk_index in 0..PXS_MAX_CHUNK {
            if self.chunks[chunk_index].is_none() {
                self.chunks[chunk_index] = Some(vec![None; PXS_CHUNK_SIZE]);
                self.chunk_counts[chunk_index] = 0;
            }
            if self.chunk_counts[chunk_index] < PXS_CHUNK_SIZE {
                let chunk = self.chunks[chunk_index]
                    .as_mut()
                    .expect("chunk allocated above");
                if let Some(slot) = chunk.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(Pxs {
                        mat,
                        x,
                        y,
                        xdir,
                        ydir,
                    });
                    self.chunk_counts[chunk_index] += 1;
                    return true;
                }
            }
        }
        false
    }

    /// `C4PXSSystem::Cast` (C4PXS.cpp:303-316): per particle draw
    /// `Random(level+1)` for ydir FIRST, then for xdir (forced argument
    /// evaluation order), giving xdir = itofix(r1 - level/2)/10 and
    /// ydir = itofix(r2 - level)/10.
    pub fn cast(
        &mut self,
        rng: &mut LcgRng,
        mat: MaterialId,
        num: i32,
        tx: i32,
        ty: i32,
        level: i32,
    ) {
        for _ in 0..num {
            let r2 = rng.random(level + 1);
            let r1 = rng.random(level + 1);
            self.create(
                mat,
                itofix(tx),
                itofix(ty),
                C4Fixed::from_raw(itofix(r1 - level / 2).val() / 10),
                C4Fixed::from_raw(itofix(r2 - level).val() / 10),
            );
        }
    }

    /// Live PXS count (recomputed each `C4PXSSystem::Execute`, C4PXS.cpp:215).
    pub fn count(&self) -> usize {
        self.chunk_counts.iter().sum()
    }

    /// Slot accessors for the engine-driven execute loop. The engine walks
    /// chunk-major slot order like `C4PXSSystem::Execute` (C4PXS.cpp:212-234)
    /// and runs each live PXS IN PLACE: `peek_slot` copies the pixel while
    /// the slot keeps carrying it (Mat != MNone for the whole execution, so
    /// `New()` — C4PXS.cpp:195-202 — never hands the executing slot to a
    /// PXS created inside a reaction). The survivor writes back via
    /// `put_slot`; a death clears via `clear_slot`.
    pub fn peek_slot(&self, chunk: usize, slot: usize) -> Option<Pxs> {
        self.chunks
            .get(chunk)
            .and_then(|chunk| chunk.as_ref())
            .and_then(|slots| slots.get(slot))
            .and_then(|entry| *entry)
    }

    pub fn put_slot(&mut self, chunk: usize, slot: usize, pxs: Pxs) {
        if let Some(slots) = self.chunks.get_mut(chunk).and_then(|chunk| chunk.as_mut()) {
            slots[slot] = Some(pxs);
        }
    }

    /// `C4PXS::Deactivate` (C4PXS.cpp:139-149): Mat = MNone plus the chunk
    /// count decrement (`C4PXSSystem::Delete`, C4PXS.cpp:426-437).
    pub fn clear_slot(&mut self, chunk: usize, slot: usize) {
        let cleared = self
            .chunks
            .get_mut(chunk)
            .and_then(|chunk| chunk.as_mut())
            .and_then(|slots| slots.get_mut(slot))
            .and_then(|entry| entry.take())
            .is_some();
        if cleared {
            if let Some(count) = self.chunk_counts.get_mut(chunk) {
                *count = count.saturating_sub(1);
            }
        }
    }

    /// Empty-chunk cleanup at the head of `C4PXSSystem::Execute`
    /// (C4PXS.cpp:218-222).
    pub fn free_empty_chunks(&mut self) {
        self.ensure_layout();
        for chunk_index in 0..PXS_MAX_CHUNK {
            if self.chunks[chunk_index].is_some() && self.chunk_counts[chunk_index] == 0 {
                self.chunks[chunk_index] = None;
            }
        }
    }

    pub fn chunk_allocated(&self, chunk: usize) -> bool {
        self.chunks
            .get(chunk)
            .map(|chunk| chunk.is_some())
            .unwrap_or(false)
    }

    /// All live PXS in execution order (snapshot/save).
    pub fn iter(&self) -> impl Iterator<Item = &Pxs> {
        self.chunks
            .iter()
            .flatten()
            .flat_map(|slots| slots.iter().flatten())
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.chunk_counts.clear();
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::fixtoi;

    fn mat(index: usize) -> MaterialId {
        MaterialId::new(index).expect("valid material id")
    }

    fn fixed(v: i32) -> C4Fixed {
        itofix(v)
    }

    #[test]
    fn create_fills_lowest_free_slot_like_cpp_new() {
        // C4PXSSystem::New (C4PXS.cpp:175-199): first chunk with space wins,
        // and within it the first Mat==MNone slot — freed slots are reused
        // at the lowest index, which feeds the deterministic execution order.
        let mut system = PxsSystem::default();
        for i in 0..3 {
            assert!(system.create(mat(0), fixed(i), fixed(0), fixed(0), fixed(0)));
        }
        assert_eq!(system.count(), 3);

        // free the middle slot (slot 1)
        let removed = system.peek_slot(0, 1).expect("slot 1 live");
        system.clear_slot(0, 1);
        assert_eq!(fixtoi(removed.x), 1);
        assert_eq!(system.count(), 2);

        // next create reuses slot 1, not slot 3
        assert!(system.create(mat(0), fixed(99), fixed(0), fixed(0), fixed(0)));
        let order: Vec<i32> = system.iter().map(|pxs| fixtoi(pxs.x)).collect();
        assert_eq!(order, [0, 99, 2], "reused slot keeps chunk-major order");
    }

    #[test]
    fn create_overflows_into_next_chunk_and_respects_global_cap() {
        // 20 chunks × 500 slots (C4PXS.h:40); a full chunk is skipped via its
        // count (C4PXS.cpp:189) and creation fails once all chunks are full.
        let mut system = PxsSystem::default();
        for i in 0..(PXS_CHUNK_SIZE + 1) {
            assert!(system.create(mat(0), fixed(i as i32), fixed(0), fixed(0), fixed(0)));
        }
        assert!(system.chunk_allocated(0));
        assert!(system.chunk_allocated(1));
        assert_eq!(system.count(), PXS_CHUNK_SIZE + 1);

        for _ in (PXS_CHUNK_SIZE + 1)..(PXS_MAX_CHUNK * PXS_CHUNK_SIZE) {
            assert!(system.create(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)));
        }
        assert!(
            !system.create(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)),
            "all 10000 slots full → create fails (C4PXS.cpp:198)"
        );
    }

    #[test]
    fn free_empty_chunks_releases_storage_like_cpp_execute() {
        // C4PXSSystem::Execute frees allocated chunks whose count hit zero
        // (C4PXS.cpp:218-222).
        let mut system = PxsSystem::default();
        assert!(system.create(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)));
        system.clear_slot(0, 0);
        assert!(system.chunk_allocated(0));
        system.free_empty_chunks();
        assert!(!system.chunk_allocated(0));
    }

    #[test]
    fn cast_consumes_synced_draws_in_cpp_order() {
        // C4PXSSystem::Cast (C4PXS.cpp:303-315): r2 = Random(level+1) drawn
        // BEFORE r1 (forced evaluation order); xdir = itofix(r1-level/2)/10,
        // ydir = itofix(r2-level)/10 — raw int division on the fixed value.
        let mut system = PxsSystem::default();
        let mut rng = LcgRng::new(42);
        let mut mirror = LcgRng::new(42);
        system.cast(&mut rng, mat(2), 2, 30, 40, 20);
        assert_eq!(rng, {
            mirror.random(21);
            mirror.random(21);
            mirror.random(21);
            mirror.random(21);
            mirror.clone()
        });

        let mut mirror = LcgRng::new(42);
        let particles: Vec<&Pxs> = system.iter().collect();
        assert_eq!(particles.len(), 2);
        for pxs in particles {
            let r2 = mirror.random(21);
            let r1 = mirror.random(21);
            assert_eq!(pxs.mat, mat(2));
            assert_eq!(fixtoi(pxs.x), 30);
            assert_eq!(fixtoi(pxs.y), 40);
            assert_eq!(pxs.xdir, C4Fixed::from_raw(itofix(r1 - 10).val() / 10));
            assert_eq!(pxs.ydir, C4Fixed::from_raw(itofix(r2 - 20).val() / 10));
        }
    }
}
