//! Port of the C4PXS pixel-sprite system storage (src/C4PXS.{h,cpp}).
//!
//! PXS are sync-relevant: they carry `C4Fixed` state, consume the synced
//! `Random()` stream (wind drift, casts), and react with the landscape. The
//! chunk/slot storage is part of observable behavior — `New()` fills the
//! lowest free slot and `Execute()` iterates chunks then slots in order, so
//! slot reuse determines deterministic execution order.

use crate::math::{itofix, C4Fixed, FixedVec2};
use crate::rng::LcgRng;
use crate::MaterialId;
use serde::{Deserialize, Serialize};

/// `PXSChunkSize` / `PXSMaxChunk` (C4PXS.h:40).
pub const PXS_CHUNK_SIZE: usize = 500;
pub const PXS_MAX_CHUNK: usize = 20;
const PXS_RECORD_BYTES: usize = 5 * std::mem::size_of::<i32>();
const PXS_CHUNK_BYTES: usize = PXS_CHUNK_SIZE * PXS_RECORD_BYTES;
const M_NONE: i32 = -1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PxsComponentError {
    #[error("PXS.c4b has invalid byte length {0}")]
    InvalidSize(usize),
    #[error("PXS.c4b uses unsupported numeric format {0}")]
    InvalidNumberFormat(i32),
    #[error("PXS.c4b contains {0} chunks; maximum is {PXS_MAX_CHUNK}")]
    TooManyChunks(usize),
    #[error("PXS.c4b contains unrepresentable material index {0}")]
    InvalidMaterial(i32),
}

fn read_component_i32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().expect("four-byte component field"))
}

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
    /// `C4PXSSystem::Count`: number of occupied slots encountered by the
    /// latest Execute pass, including pixels that deactivated while running.
    execute_count: usize,
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

    /// Draw one cast velocity in the forced C++ evaluation order. Kept
    /// separate from storage so script-host CastPXS can sample immediately
    /// while deferring the actual PXS insertion to the engine outcome fold.
    pub(crate) fn sample_cast_velocity(rng: &mut LcgRng, level: i32) -> FixedVec2 {
        let r2 = rng.random(level + 1);
        let r1 = rng.random(level + 1);
        FixedVec2::new(
            C4Fixed::from_raw(itofix(r1 - level / 2).val() / 10),
            C4Fixed::from_raw(itofix(r2 - level).val() / 10),
        )
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
            let velocity = Self::sample_cast_velocity(rng, level);
            self.create(
                mat,
                itofix(tx),
                itofix(ty),
                velocity.x,
                velocity.y,
            );
        }
    }

    /// Current live-slot count (`iChunkPXS` summed across chunks), distinct
    /// from C++'s per-Execute public `Count` ledger.
    pub fn count(&self) -> usize {
        self.chunk_counts.iter().sum()
    }

    /// `C4PXSSystem::Count` as observed by `C4ControlSyncCheck::Set` after
    /// the frame's Execute pass (C4PXS.cpp:212-234; C4Control.cpp:453).
    pub fn execute_count(&self) -> usize {
        self.execute_count
    }

    pub(crate) fn begin_execute(&mut self) {
        self.execute_count = 0;
    }

    pub(crate) fn note_executed(&mut self) {
        self.execute_count = self.execute_count.saturating_add(1);
    }

    /// `C4PXSSystem::Synchronize` (C4PXS.cpp:401-404).
    pub(crate) fn synchronize(&mut self) {
        self.execute_count = 0;
    }

    pub(crate) fn set_execute_count(&mut self, count: usize) {
        self.execute_count = count;
    }

    /// Safely compact the chunks in `C4PXSSystem::SyncClearance` order.
    ///
    /// The C++ loop copies a surviving pointer downward but never nulls its
    /// moved-from slot (C4PXS.cpp:406-424). A gap before a live chunk therefore
    /// aliases one allocation from two slots; it can be executed twice and
    /// later `delete[]`d twice. Rust deliberately diverges from that undefined
    /// behavior: `take` transfers unique ownership and the tail is cleared.
    pub(crate) fn sync_clearance(&mut self) {
        self.ensure_layout();
        let mut destination = 0;
        for source in 0..PXS_MAX_CHUNK {
            let count = self.chunk_counts[source];
            let chunk = self.chunks[source].take();
            if count == 0 {
                continue;
            }
            let Some(chunk) = chunk else {
                continue;
            };
            self.chunks[destination] = Some(chunk);
            self.chunk_counts[destination] = count;
            destination += 1;
        }
        for index in destination..PXS_MAX_CHUNK {
            self.chunks[index] = None;
            self.chunk_counts[index] = 0;
        }
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

    /// Live PXS with their SAVED slot coordinates: `C4PXSSystem::Save`
    /// writes every allocated chunk consecutively — gaps included
    /// (C4PXS.cpp:346-349) — so the saved chunk index is the chunk's rank
    /// among allocated chunks and slots keep their in-chunk position.
    pub fn iter_slots(&self) -> impl Iterator<Item = (usize, usize, &Pxs)> {
        self.chunks
            .iter()
            .filter_map(|chunk| chunk.as_ref())
            .enumerate()
            .flat_map(|(chunk_rank, slots)| {
                slots.iter().enumerate().filter_map(move |(slot, entry)| {
                    entry.as_ref().map(|pxs| (chunk_rank, slot, pxs))
                })
            })
    }

    /// Place a loaded PXS at its saved slot (`C4PXSSystem::Load` reads
    /// chunks verbatim and counts pixels in place, C4PXS.cpp:383-397).
    pub fn create_at(&mut self, chunk: usize, slot: usize, pxs: Pxs) -> bool {
        if chunk >= PXS_MAX_CHUNK || slot >= PXS_CHUNK_SIZE {
            return false;
        }
        self.ensure_layout();
        let slots = self.chunks[chunk].get_or_insert_with(|| vec![None; PXS_CHUNK_SIZE]);
        if slots[slot].replace(pxs).is_none() {
            self.chunk_counts[chunk] += 1;
        }
        true
    }

    /// Decode the raw `PXS.c4b` component written by `C4PXSSystem::Save`.
    /// Modern files start with numeric-format tag 1; untagged files are also
    /// fixed-point, while historical tag 2 stores four native IEEE floats.
    pub(crate) fn from_c4b(bytes: &[u8]) -> Result<Self, PxsComponentError> {
        let (number_format, payload) = if bytes.len() % PXS_CHUNK_BYTES == 4 {
            let number_format = read_component_i32(&bytes[..4]);
            if !(1..=2).contains(&number_format) {
                return Err(PxsComponentError::InvalidNumberFormat(number_format));
            }
            (number_format, &bytes[4..])
        } else if bytes.len() % PXS_CHUNK_BYTES == 0 {
            (1, bytes)
        } else {
            return Err(PxsComponentError::InvalidSize(bytes.len()));
        };
        let chunk_count = payload.len() / PXS_CHUNK_BYTES;
        if chunk_count > PXS_MAX_CHUNK {
            return Err(PxsComponentError::TooManyChunks(chunk_count));
        }

        let mut system = Self::default();
        system.ensure_layout();
        for chunk_index in 0..chunk_count {
            system.chunks[chunk_index] = Some(vec![None; PXS_CHUNK_SIZE]);
            let chunk_start = chunk_index * PXS_CHUNK_BYTES;
            for slot in 0..PXS_CHUNK_SIZE {
                let record_start = chunk_start + slot * PXS_RECORD_BYTES;
                let record = &payload[record_start..record_start + PXS_RECORD_BYTES];
                let raw_material = read_component_i32(&record[..4]);
                if raw_material == M_NONE {
                    continue;
                }
                let material = usize::try_from(raw_material)
                    .ok()
                    .and_then(MaterialId::new)
                    .ok_or(PxsComponentError::InvalidMaterial(raw_material))?;
                let fixed = |field: usize| {
                    let raw = read_component_i32(&record[field * 4..field * 4 + 4]);
                    if number_format == 2 {
                        crate::math::ftofix(f32::from_bits(raw as u32))
                    } else {
                        C4Fixed::from_raw(raw)
                    }
                };
                system.chunks[chunk_index]
                    .as_mut()
                    .expect("chunk allocated above")[slot] = Some(Pxs {
                    mat: material,
                    x: fixed(1),
                    y: fixed(2),
                    xdir: fixed(3),
                    ydir: fixed(4),
                });
                system.chunk_counts[chunk_index] += 1;
            }
        }
        Ok(system)
    }

    /// Encode the modern fixed-point `PXS.c4b` form. `None` matches C++
    /// Save's deletion of the component when no live PXS exists.
    pub(crate) fn to_c4b(&self) -> Option<Vec<u8>> {
        if self.count() == 0 {
            return None;
        }
        let allocated_chunks = self.chunks.iter().filter(|chunk| chunk.is_some()).count();
        let mut bytes = Vec::with_capacity(4 + allocated_chunks * PXS_CHUNK_BYTES);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        for chunk in self.chunks.iter().filter_map(Option::as_ref) {
            for slot in chunk {
                match slot {
                    Some(pxs) => {
                        bytes.extend_from_slice(&(pxs.mat.index() as i32).to_le_bytes());
                        for value in [pxs.x, pxs.y, pxs.xdir, pxs.ydir] {
                            bytes.extend_from_slice(&value.val().to_le_bytes());
                        }
                    }
                    None => {
                        bytes.extend_from_slice(&M_NONE.to_le_bytes());
                        bytes.extend_from_slice(&[0; PXS_RECORD_BYTES - 4]);
                    }
                }
            }
        }
        Some(bytes)
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.chunk_counts.clear();
        self.execute_count = 0;
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
    fn sync_clearance_compacts_once_in_cpp_survivor_order() {
        // Pin the accepted safety divergence: preserve C++ survivor order,
        // but clear every moved-from slot instead of retaining duplicate
        // owning pointers (pristine 9ffa0a5d src/C4PXS.cpp:406-424).
        let mut system = PxsSystem::default();
        assert!(system.create_at(
            0,
            0,
            Pxs {
                mat: mat(0),
                x: fixed(1),
                y: fixed(0),
                xdir: fixed(0),
                ydir: fixed(0),
            },
        ));
        assert!(system.create_at(
            1,
            4,
            Pxs {
                mat: mat(0),
                x: fixed(11),
                y: fixed(0),
                xdir: fixed(0),
                ydir: fixed(0),
            },
        ));
        assert!(system.create_at(
            3,
            2,
            Pxs {
                mat: mat(0),
                x: fixed(33),
                y: fixed(0),
                xdir: fixed(0),
                ydir: fixed(0),
            },
        ));
        system.clear_slot(0, 0);

        system.sync_clearance();

        assert!(system.chunk_allocated(0));
        assert!(system.chunk_allocated(1));
        assert!(!system.chunk_allocated(2));
        assert!(!system.chunk_allocated(3));
        assert_eq!(system.count(), 2);
        assert_eq!(
            system.iter().map(|pxs| fixtoi(pxs.x)).collect::<Vec<_>>(),
            [11, 33]
        );
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

    const CPP_PXS_FORM1: &[u8] = include_bytes!("../tests/fixtures/cpp_pxs_form1.c4b");
    const CPP_PXS_FORM2: &[u8] = include_bytes!("../tests/fixtures/cpp_pxs_form2.c4b");

    #[test]
    fn c4b_load_restores_fixed_and_historical_float_chunks_and_slots() {
        for fixture in [CPP_PXS_FORM1, CPP_PXS_FORM2] {
            let system = PxsSystem::from_c4b(fixture)
                .expect("C++ PXS component loads");
            let slots = system.iter_slots().collect::<Vec<_>>();
            assert_eq!(slots.len(), 2);
            assert_eq!((slots[0].0, slots[0].1, slots[0].2.mat), (0, 7, mat(2)));
            assert_eq!((slots[1].0, slots[1].1, slots[1].2.mat), (2, 499, mat(4)));
            assert!(system.chunk_allocated(1), "serialized empty chunks stay allocated");
            let expected = [78_643, -327_680, 32_768, -6_553].map(C4Fixed::from_raw);
            assert_eq!(
                [slots[0].2.x, slots[0].2.y, slots[0].2.xdir, slots[0].2.ydir],
                expected
            );

            let modern = system.to_c4b().expect("live PXS component saves");
            assert_eq!(read_component_i32(&modern[..4]), 1);
            assert_eq!(modern.len(), 30_004);
            assert_eq!(modern, CPP_PXS_FORM1);
            let restored = PxsSystem::from_c4b(&modern).expect("saved component reloads");
            assert_eq!(
                restored
                    .iter_slots()
                    .map(|(chunk, slot, pxs)| (chunk, slot, *pxs))
                    .collect::<Vec<_>>(),
                system
                    .iter_slots()
                    .map(|(chunk, slot, pxs)| (chunk, slot, *pxs))
                    .collect::<Vec<_>>()
            );
        }
        let untagged = PxsSystem::from_c4b(&CPP_PXS_FORM1[4..])
            .expect("legacy untagged fixed-point component loads");
        assert_eq!(untagged.peek_slot(2, 499).map(|pxs| pxs.mat), Some(mat(4)));
    }

    #[test]
    fn c4b_load_rejects_invalid_tag_size_and_chunk_count() {
        assert_eq!(
            PxsSystem::from_c4b(&3i32.to_le_bytes()).unwrap_err(),
            PxsComponentError::InvalidNumberFormat(3)
        );
        assert_eq!(
            PxsSystem::from_c4b(&[0]).unwrap_err(),
            PxsComponentError::InvalidSize(1)
        );
        let oversized = vec![0; (PXS_MAX_CHUNK + 1) * PXS_CHUNK_BYTES];
        assert_eq!(
            PxsSystem::from_c4b(&oversized).unwrap_err(),
            PxsComponentError::TooManyChunks(PXS_MAX_CHUNK + 1)
        );
    }
}
