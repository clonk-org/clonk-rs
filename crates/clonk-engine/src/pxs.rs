//! Port of the C4PXS pixel-sprite system storage (src/C4PXS.{h,cpp}).
//!
//! PXS are sync-relevant: they carry `C4Fixed` state, consume the synced
//! `Random()` stream (wind drift, casts), and react with the landscape. The
//! chunk/slot storage is part of observable behavior — `New()` fills the
//! lowest free slot and `Execute()` iterates chunks then slots in order, so
//! slot reuse determines deterministic execution order.

use crate::math::{itofix, C4Fixed, FixedVec2};
use crate::rng::LcgRng;
use crate::{MaterialId, MaterialSet};
use serde::{Deserialize, Serialize};

/// `PXSChunkSize` / `PXSMaxChunk` (C4PXS.h:40).
/// What one PXS Execute pass walks, against what it finds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PxsScanBaseline {
    pub allocated_chunks: usize,
    /// Slots the previous pass actually inspected, live or empty.
    pub visited_slots: usize,
    /// Live PXS the previous pass executed.
    pub live: usize,
}

impl PxsScanBaseline {
    /// Visited slots that held nothing.
    pub fn scanned_empty(&self) -> usize {
        self.visited_slots.saturating_sub(self.live)
    }
}

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
}

fn read_component_i32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().expect("four-byte component field"))
}

/// `C4PXS::Mat` (C4PXS.h:33): a raw material index, not a validated handle.
///
/// `C4PXSSystem::Load` stores whatever the file held and `mrfScript` writes
/// back whatever the reaction returned, both without checking the range
/// (C4PXS.cpp:362-404; C4Material.cpp:822). Only `C4PXS::Execute`'s
/// `MatValid` guard rejects an out-of-range value, one tick later
/// (C4PXS.cpp:46-50; C4Wrappers.h:100-103) — and until it does, the slot
/// stays occupied, which is what `New` allocates around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PxsMaterial(i32);

impl PxsMaterial {
    pub fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> i32 {
        self.0
    }

    /// The engine-side handle, when the raw index can carry one at all. A
    /// `Some` here is still only a representable index — `MatValid`'s upper
    /// bound is the live map's size, so callers filter through `MaterialSet`.
    pub fn id(self) -> Option<MaterialId> {
        usize::try_from(self.0).ok().and_then(MaterialId::new)
    }
}

impl From<MaterialId> for PxsMaterial {
    fn from(id: MaterialId) -> Self {
        Self(id.index() as i32)
    }
}

impl PartialEq<MaterialId> for PxsMaterial {
    fn eq(&self, other: &MaterialId) -> bool {
        self.0 == other.index() as i32
    }
}

/// One pixel sprite (C4PXS.h:25-38). `Mat == MNone` (a free slot) is modeled
/// as `None` in the chunk arrays instead of a sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pxs {
    pub mat: PxsMaterial,
    pub x: C4Fixed,
    pub y: C4Fixed,
    pub xdir: C4Fixed,
    pub ydir: C4Fixed,
}

/// The non-material fields C++ leaves behind when a PXS deactivates. These
/// bits still reach `PXS.c4b` because Save writes complete allocated chunks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeadPxsPayload {
    x: i32,
    y: i32,
    xdir: i32,
    ydir: i32,
}

impl From<Pxs> for DeadPxsPayload {
    fn from(pxs: Pxs) -> Self {
        Self {
            x: pxs.x.val(),
            y: pxs.y.val(),
            xdir: pxs.xdir.val(),
            ydir: pxs.ydir.val(),
        }
    }
}

/// Ascending occupancy index over the PXS slot array.
///
/// An accelerator, never an authority: every bit mirrors one slot of `chunks`,
/// and the answers it gives are the ones the linear scans gave. It exists so
/// `C4PXSSystem::Execute` (C4PXS.cpp:218-240) and the inner free-slot scan of
/// `C4PXSSystem::New` (C4PXS.cpp:189-196) stop reading the empties between
/// live pixels. The visit order is unchanged: the index is read fresh at every
/// step, so a pixel created into a not-yet-visited slot is still executed in
/// the same pass and one created below the cursor is still missed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SlotOccupancy {
    /// One bit per slot, chunk-major: `chunk * PXS_CHUNK_SIZE + slot`.
    words: Vec<u64>,
}

impl SlotOccupancy {
    const SLOTS: usize = PXS_MAX_CHUNK * PXS_CHUNK_SIZE;
    const WORDS: usize = Self::SLOTS.div_ceil(u64::BITS as usize);

    fn ensure(&mut self) {
        if self.words.len() != Self::WORDS {
            self.words.resize(Self::WORDS, 0);
        }
    }

    fn set(&mut self, slot: usize) {
        if slot < Self::SLOTS {
            self.ensure();
            self.words[slot / 64] |= 1_u64 << (slot % 64);
        }
    }

    fn unset(&mut self, slot: usize) {
        if let Some(word) = self.words.get_mut(slot / 64) {
            *word &= !(1_u64 << (slot % 64));
        }
    }

    /// Where an ascending scan arrives next: the first live slot at or after
    /// `slot`, or `None` once the array is exhausted.
    fn next_live_at_or_after(&self, slot: usize) -> Option<usize> {
        (slot < Self::SLOTS)
            .then(|| {
                let mut index = slot / 64;
                let mut word = self.words.get(index).copied()? & (!0_u64 << (slot % 64));
                loop {
                    if word != 0 {
                        let found = index * 64 + word.trailing_zeros() as usize;
                        return (found < Self::SLOTS).then_some(found);
                    }
                    index += 1;
                    word = self.words.get(index).copied()?;
                }
            })
            .flatten()
    }

    /// The slot `C4PXSSystem::New`'s inner scan would take: the chunk's first
    /// free slot, or `None` when it is full.
    fn first_free_in_chunk(&self, chunk: usize) -> Option<usize> {
        let base = chunk * PXS_CHUNK_SIZE;
        let end = base + PXS_CHUNK_SIZE;
        let mut slot = base;
        while slot < end {
            let offset = slot % 64;
            let word = self.words.get(slot / 64).copied().unwrap_or(0);
            // Slots before `offset` belong to an earlier chunk position, so
            // mask them occupied rather than let them win the scan.
            let free = !(word | ((1_u64 << offset) - 1));
            if free != 0 {
                let found = slot - offset + free.trailing_zeros() as usize;
                return (found < end).then_some(found - base);
            }
            slot = slot - offset + 64;
        }
        None
    }

    fn clear_chunk(&mut self, chunk: usize) {
        let base = chunk * PXS_CHUNK_SIZE;
        (base..base + PXS_CHUNK_SIZE).for_each(|slot| self.unset(slot));
    }

    /// Rebuild from the canonical array, for every path that replaces it
    /// wholesale rather than slot by slot.
    fn rebuild(chunks: &[Option<Vec<Option<Pxs>>>]) -> Self {
        let mut index = Self::default();
        index.ensure();
        for (chunk, slots) in chunks.iter().enumerate() {
            let live = slots.iter().flat_map(|slots| slots.iter().enumerate());
            for (slot, entry) in live.filter(|(_, entry)| entry.is_some()) {
                index.set(chunk * PXS_CHUNK_SIZE + slot);
            }
        }
        index
    }
}

/// Mirror of `C4PXSSystem` (C4PXS.h:42-70) minus drawing.
#[derive(Debug, Clone, Default)]
pub struct PxsSystem {
    chunks: Vec<Option<Vec<Option<Pxs>>>>,
    /// Raw x/y/xdir/ydir bits retained for free slots in each allocated chunk.
    dead_payload_chunks: Vec<Option<Vec<DeadPxsPayload>>>,
    chunk_counts: Vec<usize>,
    /// Accelerator over `chunks`, never an authority — see `SlotOccupancy`.
    occupancy: SlotOccupancy,
    /// `C4PXSSystem::Count`: number of occupied slots encountered by the
    /// latest Execute pass, including pixels that deactivated while running.
    execute_count: usize,
    /// Slots the latest Execute pass inspected. Diagnostic only — nothing in
    /// the simulation reads it.
    inspected_slots: usize,
}

impl PxsSystem {
    fn ensure_layout(&mut self) {
        if self.chunks.len() != PXS_MAX_CHUNK {
            self.chunks.resize_with(PXS_MAX_CHUNK, || None);
            self.chunk_counts.resize(PXS_MAX_CHUNK, 0);
        }
        if self.dead_payload_chunks.len() != PXS_MAX_CHUNK {
            self.dead_payload_chunks.resize_with(PXS_MAX_CHUNK, || None);
        }
        self.occupancy.ensure();
    }

    /// `C4PXSSystem::Create` then `New` (C4PXS.cpp:181-215): reject material
    /// IDs outside the supplied live map before scanning chunks; within the
    /// first non-full chunk, allocate its first free slot and initialize the
    /// fields.
    pub(crate) fn create(
        &mut self,
        materials: &MaterialSet,
        mat: MaterialId,
        x: C4Fixed,
        y: C4Fixed,
        xdir: C4Fixed,
        ydir: C4Fixed,
    ) -> bool {
        if materials.get_by_id(mat).is_none() {
            return false;
        }
        self.create_unchecked(mat, x, y, xdir, ydir)
    }

    /// Raw `C4PXSSystem::New` allocation used by storage tests after their
    /// material validity has already been established elsewhere.
    pub(crate) fn create_unchecked(
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
                self.dead_payload_chunks[chunk_index] =
                    Some(vec![DeadPxsPayload::default(); PXS_CHUNK_SIZE]);
                self.chunk_counts[chunk_index] = 0;
                self.occupancy.clear_chunk(chunk_index);
            }
            if self.chunk_counts[chunk_index] < PXS_CHUNK_SIZE {
                if let Some(slot) = self.occupancy.first_free_in_chunk(chunk_index) {
                    let pxs = Pxs {
                        mat: mat.into(),
                        x,
                        y,
                        xdir,
                        ydir,
                    };
                    self.chunks[chunk_index]
                        .as_mut()
                        .expect("chunk allocated above")[slot] = Some(pxs);
                    self.dead_payload_chunks[chunk_index]
                        .as_mut()
                        .expect("payload chunk allocated above")[slot] = pxs.into();
                    self.chunk_counts[chunk_index] += 1;
                    self.occupancy.set(chunk_index * PXS_CHUNK_SIZE + slot);
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

    /// `C4PXSSystem::Cast` (C4PXS.cpp:309-321): per particle draw
    /// `Random(level+1)` for ydir FIRST, then for xdir (forced argument
    /// evaluation order), giving xdir = itofix(r1 - level/2)/10 and
    /// ydir = itofix(r2 - level)/10.
    pub(crate) fn cast(
        &mut self,
        materials: &MaterialSet,
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
                materials,
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

    /// What the last Execute pass walked, against the live PXS it found.
    ///
    /// `C4PXSSystem::Execute` visits every slot of every allocated chunk in
    /// chunk-major order (`C4PXS.cpp:218-240`), so the visited count could be
    /// *derived* from the allocated chunks. Deriving it is what left it
    /// unfalsifiable — nothing checked a pass agreed — so the pass reports
    /// what it inspected instead. The gap between visited and live is the work
    /// an ordered occupancy index removes.
    ///
    /// Observation only: this reads state the pass already keeps and changes
    /// no slot order.
    pub fn execute_scan_baseline(&self) -> PxsScanBaseline {
        PxsScanBaseline {
            allocated_chunks: (0..PXS_MAX_CHUNK)
                .filter(|chunk| self.chunk_allocated(*chunk))
                .count(),
            visited_slots: self.inspected_slots,
            live: self.execute_count,
        }
    }

    /// Slots the last Execute pass inspected, live or empty.
    pub fn inspected_slots(&self) -> usize {
        self.inspected_slots
    }

    /// Where the ascending Execute walk arrives next, as a flat slot index
    /// (`chunk * PXS_CHUNK_SIZE + slot`). Read fresh at every step of the
    /// pass, so mid-pass creation is seen exactly as the linear scan saw it.
    pub fn next_live_slot(&self, from: usize) -> Option<usize> {
        self.occupancy.next_live_at_or_after(from)
    }

    /// Whether the accelerator still agrees with the canonical array.
    ///
    /// The index is only ever a mirror, so any disagreement is a bug in a path
    /// that writes slots — this is what the model test asserts after each step.
    pub fn occupancy_matches_slots(&self) -> bool {
        // `ensure` first: an untouched system carries no words at all, which
        // is the same empty index a rebuild spells as all-zero words.
        let mut mirror = self.occupancy.clone();
        mirror.ensure();
        mirror == SlotOccupancy::rebuild(&self.chunks)
    }

    /// The free-slot answer read straight off the array, kept as the reference
    /// the indexed lookup is checked against (`C4PXS.cpp:189-196`).
    #[cfg(test)]
    fn first_free_in_chunk_linear(&self, chunk: usize) -> Option<usize> {
        // `New` allocates a missing chunk before scanning it
        // (C4PXS.cpp:181-188), so an absent chunk is an entirely free one.
        self.chunks
            .get(chunk)
            .and_then(|slots| slots.as_ref())
            .map_or(Some(0), |slots| slots.iter().position(Option::is_none))
    }

    /// The ascending walk read straight off the array, likewise.
    #[cfg(test)]
    fn live_slots_linear(&self) -> Vec<usize> {
        (0..PXS_MAX_CHUNK)
            .flat_map(|chunk| (0..PXS_CHUNK_SIZE).map(move |slot| (chunk, slot)))
            .filter(|(chunk, slot)| self.peek_slot(*chunk, *slot).is_some())
            .map(|(chunk, slot)| chunk * PXS_CHUNK_SIZE + slot)
            .collect()
    }

    pub(crate) fn note_inspected_slots(&mut self, inspected: usize) {
        self.inspected_slots = inspected;
    }

    /// `C4PXSSystem::Count` as observed by `C4ControlSyncCheck::Set` after
    /// the frame's Execute pass (C4PXS.cpp:218-240; C4Control.cpp:453).
    pub fn execute_count(&self) -> usize {
        self.execute_count
    }

    pub(crate) fn begin_execute(&mut self) {
        self.execute_count = 0;
        self.inspected_slots = 0;
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
            let dead_payloads = self.dead_payload_chunks[source].take();
            if count == 0 {
                continue;
            }
            let Some(chunk) = chunk else {
                continue;
            };
            self.chunks[destination] = Some(chunk);
            self.dead_payload_chunks[destination] = dead_payloads;
            self.chunk_counts[destination] = count;
            destination += 1;
        }
        for index in destination..PXS_MAX_CHUNK {
            self.chunks[index] = None;
            self.dead_payload_chunks[index] = None;
            self.chunk_counts[index] = 0;
        }
        // Chunks moved, so every bit did: rebuild rather than track the shift.
        self.occupancy = SlotOccupancy::rebuild(&self.chunks);
    }

    /// Slot accessors for the engine-driven execute loop. The engine walks
    /// chunk-major slot order like `C4PXSSystem::Execute` (C4PXS.cpp:218-240)
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
            self.dead_payload_chunks[chunk]
                .as_mut()
                .expect("allocated PXS chunk has payload storage")[slot] = pxs.into();
            self.occupancy.set(chunk * PXS_CHUNK_SIZE + slot);
        }
    }

    /// `C4PXS::Deactivate` (C4PXS.cpp:139-149): Mat = MNone plus the chunk
    /// count decrement (`C4PXSSystem::Delete`, C4PXS.cpp:426-437).
    pub fn clear_slot(&mut self, chunk: usize, slot: usize) {
        self.clear_slot_with_payload(chunk, slot, None);
    }

    /// Clear an executing slot while retaining the fields as mutated by its
    /// final `C4PXS::Execute` path before Deactivate sets Mat to MNone.
    pub(crate) fn deactivate_slot(&mut self, chunk: usize, slot: usize, pxs: Pxs) {
        self.clear_slot_with_payload(chunk, slot, Some(pxs.into()));
    }

    fn clear_slot_with_payload(
        &mut self,
        chunk: usize,
        slot: usize,
        dead_payload: Option<DeadPxsPayload>,
    ) {
        let cleared = self
            .chunks
            .get_mut(chunk)
            .and_then(|chunk| chunk.as_mut())
            .and_then(|slots| slots.get_mut(slot))
            .and_then(|entry| entry.take());
        if let Some(pxs) = cleared {
            self.dead_payload_chunks[chunk]
                .as_mut()
                .expect("allocated PXS chunk has payload storage")[slot] =
                dead_payload.unwrap_or_else(|| pxs.into());
            self.occupancy.unset(chunk * PXS_CHUNK_SIZE + slot);
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
            self.free_empty_chunk(chunk_index);
        }
    }

    /// The outer Execute loop performs this check immediately before it scans
    /// each individual chunk (C4PXS.cpp:218-240), not as a global pre-pass.
    pub(crate) fn free_empty_chunk(&mut self, chunk_index: usize) {
        self.ensure_layout();
        if chunk_index < PXS_MAX_CHUNK
            && self.chunks[chunk_index].is_some()
            && self.chunk_counts[chunk_index] == 0
        {
            self.chunks[chunk_index] = None;
            self.dead_payload_chunks[chunk_index] = None;
            self.occupancy.clear_chunk(chunk_index);
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
        let dead_payloads = self.dead_payload_chunks[chunk]
            .get_or_insert_with(|| vec![DeadPxsPayload::default(); PXS_CHUNK_SIZE]);
        if slots[slot].replace(pxs).is_none() {
            self.chunk_counts[chunk] += 1;
        }
        dead_payloads[slot] = pxs.into();
        self.occupancy.set(chunk * PXS_CHUNK_SIZE + slot);
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
        } else if bytes.len().is_multiple_of(PXS_CHUNK_BYTES) {
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
            system.dead_payload_chunks[chunk_index] =
                Some(vec![DeadPxsPayload::default(); PXS_CHUNK_SIZE]);
            let chunk_start = chunk_index * PXS_CHUNK_BYTES;
            for slot in 0..PXS_CHUNK_SIZE {
                let record_start = chunk_start + slot * PXS_RECORD_BYTES;
                let record = &payload[record_start..record_start + PXS_RECORD_BYTES];
                let raw_material = read_component_i32(&record[..4]);
                system.dead_payload_chunks[chunk_index]
                    .as_mut()
                    .expect("payload chunk allocated above")[slot] = DeadPxsPayload {
                    x: read_component_i32(&record[4..8]),
                    y: read_component_i32(&record[8..12]),
                    xdir: read_component_i32(&record[12..16]),
                    ydir: read_component_i32(&record[16..20]),
                };
                if raw_material == M_NONE {
                    continue;
                }
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
                    mat: PxsMaterial::from_raw(raw_material),
                    x: fixed(1),
                    y: fixed(2),
                    xdir: fixed(3),
                    ydir: fixed(4),
                });
                system.chunk_counts[chunk_index] += 1;
            }
        }
        system.occupancy = SlotOccupancy::rebuild(&system.chunks);
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
        for (chunk_index, chunk) in self
            .chunks
            .iter()
            .enumerate()
            .filter_map(|(index, chunk)| chunk.as_ref().map(|chunk| (index, chunk)))
        {
            let dead_payloads = self.dead_payload_chunks[chunk_index]
                .as_ref()
                .expect("allocated PXS chunk has payload storage");
            for (slot_index, slot) in chunk.iter().enumerate() {
                match slot {
                    Some(pxs) => {
                        bytes.extend_from_slice(&pxs.mat.raw().to_le_bytes());
                        for value in [pxs.x, pxs.y, pxs.xdir, pxs.ydir] {
                            bytes.extend_from_slice(&value.val().to_le_bytes());
                        }
                    }
                    None => {
                        bytes.extend_from_slice(&M_NONE.to_le_bytes());
                        let dead = dead_payloads[slot_index];
                        for value in [dead.x, dead.y, dead.xdir, dead.ydir] {
                            bytes.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
        }
        Some(bytes)
    }

    /// `C4PXSSystem::Load` (C4PXS.cpp:362-404), in place on the live system.
    ///
    /// C++ clears the existing chunks before it validates the number format
    /// and the chunk count, so a malformed component leaves the system empty
    /// rather than untouched — and `Clear` keeps the public `Count`, which is
    /// what the sync check transmits.
    pub(crate) fn load_c4b(&mut self, bytes: &[u8]) -> Result<(), PxsComponentError> {
        self.clear();
        let count = self.execute_count;
        *self = Self::from_c4b(bytes)?;
        self.execute_count = count;
        Ok(())
    }

    /// `C4PXSSystem::Clear` (C4PXS.cpp:171-179): free every chunk and zero
    /// the per-chunk counters. The public `Count` ledger is deliberately
    /// untouched — only `Default` resets it.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.dead_payload_chunks.clear();
        self.chunk_counts.clear();
        self.occupancy = SlotOccupancy::default();
        self.inspected_slots = 0;
    }

    /// The `Clear(); Default();` pair C++ runs when a loaded landscape
    /// carries no PXS component (C4PXS.cpp:161-179; C4Game.cpp:2683-2686).
    pub(crate) fn reset_to_default(&mut self) {
        self.clear();
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
        // C4PXSSystem::New (C4PXS.cpp:181-205): first chunk with space wins,
        // and within it the first Mat==MNone slot — freed slots are reused
        // at the lowest index, which feeds the deterministic execution order.
        let mut system = PxsSystem::default();
        for i in 0..3 {
            assert!(system.create_unchecked(mat(0), fixed(i), fixed(0), fixed(0), fixed(0)));
        }
        assert_eq!(system.count(), 3);

        // free the middle slot (slot 1)
        let removed = system.peek_slot(0, 1).expect("slot 1 live");
        system.clear_slot(0, 1);
        assert_eq!(fixtoi(removed.x), 1);
        assert_eq!(system.count(), 2);

        // next create reuses slot 1, not slot 3
        assert!(system.create_unchecked(mat(0), fixed(99), fixed(0), fixed(0), fixed(0)));
        let order: Vec<i32> = system.iter().map(|pxs| fixtoi(pxs.x)).collect();
        assert_eq!(order, [0, 99, 2], "reused slot keeps chunk-major order");
    }

    #[test]
    fn create_overflows_into_next_chunk_and_respects_global_cap() {
        // 20 chunks × 500 slots (C4PXS.h:40); a full chunk is skipped via its
        // count (C4PXS.cpp:189) and creation fails once all chunks are full.
        let mut system = PxsSystem::default();
        for i in 0..(PXS_CHUNK_SIZE + 1) {
            assert!(system.create_unchecked(mat(0), fixed(i as i32), fixed(0), fixed(0), fixed(0)));
        }
        assert!(system.chunk_allocated(0));
        assert!(system.chunk_allocated(1));
        assert_eq!(system.count(), PXS_CHUNK_SIZE + 1);

        for _ in (PXS_CHUNK_SIZE + 1)..(PXS_MAX_CHUNK * PXS_CHUNK_SIZE) {
            assert!(system.create_unchecked(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)));
        }
        assert!(
            !system.create_unchecked(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)),
            "all 10000 slots full → create fails (C4PXS.cpp:204)"
        );
    }

    #[test]
    fn free_empty_chunks_releases_storage_like_cpp_execute() {
        // C4PXSSystem::Execute frees allocated chunks whose count hit zero
        // (C4PXS.cpp:218-222).
        let mut system = PxsSystem::default();
        assert!(system.create_unchecked(mat(0), fixed(0), fixed(0), fixed(0), fixed(0)));
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
                mat: mat(0).into(),
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
                mat: mat(0).into(),
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
                mat: mat(0).into(),
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
    fn dead_payload_storage_tracks_chunk_compaction_reuse_and_release() {
        // SyncClearance moves surviving chunks in order (C4PXS.cpp:406-424),
        // and New reuses the lowest Mat==MNone slot (C4PXS.cpp:181-205).
        let mut system = PxsSystem::default();
        let live = Pxs {
            mat: mat(0).into(),
            x: C4Fixed::from_raw(1),
            y: C4Fixed::from_raw(2),
            xdir: C4Fixed::from_raw(3),
            ydir: C4Fixed::from_raw(4),
        };
        let old_dead = Pxs {
            x: C4Fixed::from_raw(11),
            y: C4Fixed::from_raw(12),
            xdir: C4Fixed::from_raw(13),
            ydir: C4Fixed::from_raw(14),
            ..live
        };
        assert!(system.create_at(2, 0, live));
        assert!(system.create_at(2, 1, old_dead));
        system.clear_slot(2, 1);

        system.sync_clearance();

        assert_eq!(
            system.dead_payload_chunks[0]
                .as_ref()
                .expect("moved payload chunk")[1],
            old_dead.into()
        );
        let replacement = Pxs {
            x: C4Fixed::from_raw(21),
            y: C4Fixed::from_raw(22),
            xdir: C4Fixed::from_raw(23),
            ydir: C4Fixed::from_raw(24),
            ..live
        };
        assert!(system.create_unchecked(
            replacement
                .mat
                .id()
                .expect("fixture material is representable"),
            replacement.x,
            replacement.y,
            replacement.xdir,
            replacement.ydir,
        ));
        assert_eq!(system.peek_slot(0, 1), Some(replacement));
        assert_eq!(
            system.dead_payload_chunks[0]
                .as_ref()
                .expect("payload chunk")[1],
            replacement.into()
        );

        system.clear_slot(0, 0);
        system.clear_slot(0, 1);
        assert!(
            system.to_c4b().is_none(),
            "dead payloads alone do not keep PXS.c4b"
        );
        system.free_empty_chunks();
        assert!(system.dead_payload_chunks[0].is_none());
        system.clear();
        assert!(system.dead_payload_chunks.is_empty());
    }

    #[test]
    fn cast_consumes_synced_draws_in_cpp_order() {
        // C4PXSSystem::Cast (C4PXS.cpp:309-321): r2 = Random(level+1) drawn
        // BEFORE r1 (forced evaluation order); xdir = itofix(r1-level/2)/10,
        // ydir = itofix(r2-level)/10 — raw int division on the fixed value.
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=M0\n[Material]\nName=M1\n[Material]\nName=M2\n",
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut system = PxsSystem::default();
        let mut rng = LcgRng::new(42);
        let mut mirror = LcgRng::new(42);
        system.cast(&materials, &mut rng, mat(2), 2, 30, 40, 20);
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
    fn c4b_save_preserves_deactivated_slot_payload() {
        // Deactivate changes only Mat (pinned snapshot C4PXS.cpp:139-149),
        // then Save writes every field of every allocated slot verbatim
        // (pinned snapshot C4PXS.cpp:324-349).
        let dead_values = [17, -23, 0x1234_5678, i32::MIN + 9];
        let mut system = PxsSystem::default();
        assert!(system.create_at(
            0,
            0,
            Pxs {
                mat: mat(2).into(),
                x: C4Fixed::from_raw(dead_values[0]),
                y: C4Fixed::from_raw(dead_values[1]),
                xdir: C4Fixed::from_raw(dead_values[2]),
                ydir: C4Fixed::from_raw(dead_values[3]),
            },
        ));
        assert!(system.create_at(
            0,
            1,
            Pxs {
                mat: mat(4).into(),
                x: fixed(1),
                y: fixed(2),
                xdir: fixed(3),
                ydir: fixed(4),
            },
        ));

        system.clear_slot(0, 0);

        let bytes = system.to_c4b().expect("one live PXS keeps the component");
        let dead_record = &bytes[4..4 + PXS_RECORD_BYTES];
        assert_eq!(read_component_i32(&dead_record[..4]), M_NONE);
        let saved_values = std::array::from_fn(|field| {
            read_component_i32(&dead_record[(field + 1) * 4..(field + 2) * 4])
        });
        assert_eq!(saved_values, dead_values);
    }

    #[test]
    fn engine_state_restore_preserves_the_raw_pxs_component() {
        // Save writes every field of every allocated slot, including dead
        // payload (pinned snapshot C4PXS.cpp:324-349), and Load restores the
        // complete chunk records verbatim (C4PXS.cpp:383-397).
        let mut engine = crate::Engine::new();
        let dead = Pxs {
            mat: mat(2).into(),
            x: C4Fixed::from_raw(17),
            y: C4Fixed::from_raw(-23),
            xdir: C4Fixed::from_raw(0x1234_5678),
            ydir: C4Fixed::from_raw(i32::MIN + 9),
        };
        let live = Pxs {
            mat: mat(4).into(),
            x: fixed(1),
            y: fixed(2),
            xdir: fixed(3),
            ydir: fixed(4),
        };
        assert!(engine.pxs_system.create_at(0, 0, dead));
        assert!(engine.pxs_system.create_at(0, 1, live));
        engine.pxs_system.clear_slot(0, 0);
        let component = engine
            .pxs_system
            .to_c4b()
            .expect("live PXS keeps the component");

        let mut state = engine.capture_state();
        let projected = state
            .particles
            .iter_mut()
            .find(|particle| particle.definition_id.starts_with("material/pxs/"))
            .expect("live PXS has a legacy particle projection");
        projected.pxs_fixed = Some([101, 102, 103, 104]);
        let state: crate::EngineState =
            serde_json::from_slice(&serde_json::to_vec(&state).expect("engine state serializes"))
                .expect("engine state deserializes");
        assert_eq!(state.pxs_component.as_deref(), Some(component.as_slice()));

        let mut restored = crate::Engine::new();
        restored.restore_state(&state).expect("state restores");

        // The raw component is authoritative when present; the deliberately
        // corrupted legacy projection above must not replace its live slot.
        assert_eq!(restored.pxs_system.to_c4b(), Some(component));
    }

    #[test]
    fn c4b_load_keeps_format2_dead_payload_unconverted() {
        // The pinned Load converts format-2 fields only inside Mat != MNone
        // (C4PXS.cpp:383-397), and the next Save writes the dead bytes too
        // (C4PXS.cpp:324-349).
        let mut format2 = vec![0; 4 + PXS_CHUNK_BYTES];
        format2[..4].copy_from_slice(&2i32.to_le_bytes());
        let write_field = |bytes: &mut [u8], slot: usize, field: usize, value: i32| {
            let start = 4 + slot * PXS_RECORD_BYTES + field * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        };
        for slot in 0..PXS_CHUNK_SIZE {
            write_field(&mut format2, slot, 0, M_NONE);
        }
        write_field(&mut format2, 0, 0, 2);
        for (field, value) in [1.25_f32, -2.5, 0.5, -0.25].into_iter().enumerate() {
            write_field(&mut format2, 0, field + 1, value.to_bits() as i32);
        }
        let dead_slot = 7;
        let dead_values = [0x7fc0_1234_u32 as i32, -11, i32::MAX, i32::MIN];
        for (field, value) in dead_values.into_iter().enumerate() {
            write_field(&mut format2, dead_slot, field + 1, value);
        }

        let system = PxsSystem::from_c4b(&format2).expect("format-2 component loads");
        let saved = system.to_c4b().expect("live PXS keeps the component");
        let dead_record_start = 4 + dead_slot * PXS_RECORD_BYTES;
        let dead_record = &saved[dead_record_start..dead_record_start + PXS_RECORD_BYTES];
        let saved_values = std::array::from_fn(|field| {
            read_component_i32(&dead_record[(field + 1) * 4..(field + 2) * 4])
        });
        assert_eq!(saved_values, dead_values);
    }

    #[test]
    fn c4b_load_restores_fixed_and_historical_float_chunks_and_slots() {
        for fixture in [CPP_PXS_FORM1, CPP_PXS_FORM2] {
            let system = PxsSystem::from_c4b(fixture).expect("C++ PXS component loads");
            let slots = system.iter_slots().collect::<Vec<_>>();
            assert_eq!(slots.len(), 2);
            assert_eq!(
                (slots[0].0, slots[0].1, slots[0].2.mat),
                (0, 7, mat(2).into())
            );
            assert_eq!(
                (slots[1].0, slots[1].1, slots[1].2.mat),
                (2, 499, mat(4).into())
            );
            assert!(
                system.chunk_allocated(1),
                "serialized empty chunks stay allocated"
            );
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
        assert_eq!(
            untagged.peek_slot(2, 499).map(|pxs| pxs.mat),
            Some(mat(4).into())
        );
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

#[cfg(test)]
mod scan_baseline_tests {
    use super::*;

    /// The chunks a pass will walk, which is what `PxsSystem` alone can
    /// answer — the slots it inspects are counted by the pass itself and
    /// pinned at the engine level, where `Execute` actually runs.
    #[test]
    fn allocated_chunks_track_the_pixels_placed_in_them() {
        let mut system = PxsSystem::default();
        assert_eq!(
            system.execute_scan_baseline(),
            PxsScanBaseline::default(),
            "an empty system allocates no chunk and walks nothing"
        );

        let pixel = Pxs {
            mat: crate::TestValueExt::test_value(MaterialId::new(1)).into(),
            x: C4Fixed::from_raw(0),
            y: C4Fixed::from_raw(0),
            xdir: C4Fixed::from_raw(0),
            ydir: C4Fixed::from_raw(0),
        };
        assert!(system.create_at(0, 0, pixel));
        assert_eq!(system.execute_scan_baseline().allocated_chunks, 1);

        // A pixel in a later chunk allocates that chunk and no chunk between —
        // the sparse case clonk-org/clonk-rs#296 is about.
        assert!(system.create_at(3, 17, pixel));
        assert_eq!(system.execute_scan_baseline().allocated_chunks, 2);
    }

    /// The accelerator has to answer exactly what the linear scans answered,
    /// for every occupancy the slot array can reach.
    ///
    /// A deterministic pseudo-random walk rather than a fixed table: the
    /// failure this guards against is a chunk-boundary or word-boundary case
    /// nobody thought to enumerate — `PXS_CHUNK_SIZE` is 500, so a chunk
    /// neither starts nor ends on a 64-bit word.
    #[test]
    fn the_occupancy_index_answers_what_the_linear_scans_answer() {
        let mut system = PxsSystem::default();
        let material = crate::TestValueExt::test_value(MaterialId::new(1));
        // xorshift, so the sequence is fixed without pulling in a dependency.
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for step in 0..2_000_usize {
            let chunk = (next() as usize) % PXS_MAX_CHUNK;
            let slot = (next() as usize) % PXS_CHUNK_SIZE;
            match step % 4 {
                0 => system.clear_slot(chunk, slot),
                1 => {
                    system.create_at(
                        chunk,
                        slot,
                        Pxs {
                            mat: material.into(),
                            x: C4Fixed::from_raw(step as i32),
                            y: C4Fixed::ZERO,
                            xdir: C4Fixed::ZERO,
                            ydir: C4Fixed::ZERO,
                        },
                    );
                }
                2 => {
                    system.create_unchecked(
                        material,
                        C4Fixed::ZERO,
                        C4Fixed::ZERO,
                        C4Fixed::ZERO,
                        C4Fixed::ZERO,
                    );
                }
                _ if step % 40 == 3 => system.free_empty_chunks(),
                _ if step % 80 == 7 => system.sync_clearance(),
                _ => system.clear_slot(chunk, PXS_CHUNK_SIZE - 1 - slot),
            }

            assert!(
                system.occupancy_matches_slots(),
                "index drifted from the array at step {step}"
            );
            assert_eq!(
                system.occupancy.first_free_in_chunk(chunk),
                system.first_free_in_chunk_linear(chunk),
                "free lookup diverged at step {step}"
            );

            let indexed = {
                let mut visited = Vec::new();
                let mut cursor = 0;
                while let Some(live) = system.next_live_slot(cursor) {
                    visited.push(live);
                    cursor = live + 1;
                }
                visited
            };
            assert_eq!(
                indexed,
                system.live_slots_linear(),
                "ascending order diverged at step {step}"
            );
        }
    }

    /// Every path that replaces the slot array wholesale has to leave the
    /// accelerator agreeing with it, or a later pass skips a live pixel.
    #[test]
    fn rebuilding_paths_leave_the_index_agreeing_with_the_array() {
        let pixel = Pxs {
            mat: crate::TestValueExt::test_value(MaterialId::new(1)).into(),
            x: C4Fixed::from_raw(7),
            y: C4Fixed::from_raw(9),
            xdir: C4Fixed::ZERO,
            ydir: C4Fixed::ZERO,
        };
        let mut system = PxsSystem::default();
        for (chunk, slot) in [(0, 0), (0, 63), (0, 64), (2, PXS_CHUNK_SIZE - 1), (5, 17)] {
            assert!(system.create_at(chunk, slot, pixel));
        }

        let bytes = system.to_c4b().expect("c4b bytes");
        let loaded = PxsSystem::from_c4b(&bytes).expect("c4b load");
        assert!(loaded.occupancy_matches_slots(), "from_c4b");
        // Save renumbers chunks to their rank among the allocated ones
        // (C4PXS.cpp:346-349), so the slots move but the pixels do not.
        assert_eq!(loaded.iter().count(), system.iter().count());

        let mut compacted = system.clone();
        compacted.sync_clearance();
        assert!(compacted.occupancy_matches_slots(), "sync_clearance");

        let mut freed = system.clone();
        freed.free_empty_chunks();
        assert!(freed.occupancy_matches_slots(), "free_empty_chunks");

        let mut cleared = system;
        cleared.clear();
        assert!(cleared.occupancy_matches_slots(), "clear");
        assert_eq!(cleared.next_live_slot(0), None);
    }

    fn mat(index: usize) -> MaterialId {
        MaterialId::new(index).expect("valid material id")
    }

    /// One `PXS.c4b` chunk whose slot 0 carries `raw_material` and whose
    /// remaining slots are free.
    fn chunk_with_raw_material(raw_material: i32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + PXS_CHUNK_BYTES);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        for slot in 0..PXS_CHUNK_SIZE {
            let material = if slot == 0 { raw_material } else { M_NONE };
            bytes.extend_from_slice(&material.to_le_bytes());
            bytes.extend_from_slice(&[0u8; 16]);
        }
        bytes
    }

    /// `C4PXSSystem::Clear` frees the chunks and zeroes the per-chunk
    /// counters but leaves the public `Count` alone; only `Default` resets
    /// that ledger (oracle-src-pinned src/C4PXS.cpp:161-179). `Count` is what
    /// `C4ControlSyncCheck::Set` transmits (src/C4Control.cpp:453), so
    /// zeroing it early desyncs the check rather than the pixels.
    #[test]
    fn clear_frees_the_chunks_but_keeps_the_public_count() {
        let mut system = PxsSystem::default();
        assert!(system.create_unchecked(
            mat(1),
            C4Fixed::ZERO,
            C4Fixed::ZERO,
            C4Fixed::ZERO,
            C4Fixed::ZERO
        ));
        system.begin_execute();
        system.note_executed();
        assert_eq!(system.execute_count(), 1);

        system.clear();
        assert_eq!(system.count(), 0, "Clear frees every chunk");
        assert_eq!(
            system.execute_count(),
            1,
            "Clear does not touch the public Count ledger"
        );

        system.reset_to_default();
        assert_eq!(
            system.execute_count(),
            0,
            "Default is what resets the public Count"
        );
    }

    /// `C4PXSSystem::Load` clears the existing chunks *before* it validates
    /// the number format and the chunk count, so a malformed component
    /// leaves the system empty rather than untouched — and the cleared
    /// system keeps its public `Count` (src/C4PXS.cpp:362-381).
    #[test]
    fn a_malformed_component_leaves_the_system_cleared() {
        let mut system = PxsSystem::default();
        assert!(system.create_unchecked(
            mat(1),
            C4Fixed::ZERO,
            C4Fixed::ZERO,
            C4Fixed::ZERO,
            C4Fixed::ZERO
        ));
        system.begin_execute();
        system.note_executed();

        let mut malformed = chunk_with_raw_material(1);
        malformed[..4].copy_from_slice(&7i32.to_le_bytes());
        assert_eq!(
            system.load_c4b(&malformed),
            Err(PxsComponentError::InvalidNumberFormat(7))
        );
        assert_eq!(
            system.count(),
            0,
            "Load clears before it validates, so the failure leaves nothing"
        );
        assert_eq!(
            system.execute_count(),
            1,
            "the cleared system keeps the Count the sync check transmits"
        );
    }

    /// `C4PXSSystem::Load` accepts every record whose raw `Mat` differs from
    /// `MNone` and counts its slot; nothing there validates the index. Only
    /// `C4PXS::Execute`'s `MatValid` guard rejects an out-of-range one
    /// (oracle-src-pinned src/C4PXS.cpp:362-404 and :46-50;
    /// src/C4Wrappers.h:100-103). Refusing the record at load instead frees
    /// its slot a tick early, so the next `New` picks a different slot.
    #[test]
    fn load_keeps_a_raw_invalid_material_occupying_its_slot() {
        for raw_material in [-5, i32::MIN, 70_000, i32::MAX] {
            let system = PxsSystem::from_c4b(&chunk_with_raw_material(raw_material))
                .unwrap_or_else(|error| panic!("C++ Load accepts raw Mat {raw_material}: {error}"));
            assert_eq!(
                system.count(),
                1,
                "raw Mat {raw_material} must hold its slot until the Execute guard"
            );
            assert_eq!(
                system.peek_slot(0, 0).map(|pixel| pixel.mat.raw()),
                Some(raw_material),
                "the slot must retain the exact raw index C++ stored"
            );
        }
    }
}
