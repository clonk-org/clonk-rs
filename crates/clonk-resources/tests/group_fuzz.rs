//! Bounded malformed-input campaign over the packed C4Group reader
//! (clonk-org/clonk-rs#959).
//!
//! Scenario, definition, player, save, replay and update content all enter
//! through a group container, and a player join carries one as raw bytes over
//! the wire (`C4ControlJoinPlayer`, C4Control.cpp:731-744), so this reader sees
//! attacker-shaped input before any higher-level parser gets a say.
//!
//! The amplification risk is the header's *counts and sizes*: a 204-byte header
//! names an entry count, and each 316-byte entry names a payload size. Those are
//! numbers that multiply, so the contract is not merely "no panic" — the work
//! the reader does has to stay in proportion to the bytes it was given.
//!
//! This runs in the ordinary suite so the contract holds on every change without
//! the fuzzing engine; `fuzz/` carries the libFuzzer target for long campaigns.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use clonk_resources::Group;

/// Counts what the reader reserves, so the entry-table bound is measured rather
/// than assumed. Without it the amplification is invisible on a host whose
/// allocator hands out address space lazily: the reservation succeeds, the read
/// fails immediately after, and the test passes while a 204-byte input has
/// still asked for gigabytes.
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` and `layout` came from the wrapped allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        // SAFETY: `pointer` and `layout` came from the wrapped allocator.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// Bytes the allocator handed out while `body` ran.
fn allocated_during<T>(body: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATION_BYTES.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
    let value = body();
    COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
    (value, ALLOCATION_BYTES.load(Ordering::Relaxed))
}

const GROUP_HEADER_SIZE: usize = 204;
const GROUP_ENTRY_SIZE: usize = 316;

/// `mem_unscramble` in `group.rs`: XOR 237 over every byte, then swap each
/// aligned pair two apart. Both halves are involutions and the XOR is
/// position-independent, so the same transform scrambles and unscrambles.
fn scramble(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        *byte ^= 237;
    }
    let mut i = 0;
    while i + 2 < buffer.len() {
        buffer.swap(i, i + 2);
        i += 3;
    }
}

/// A well-formed 204-byte C4Group header declaring `entry_count` entries.
fn header(entry_count: i32) -> Vec<u8> {
    let mut bytes = vec![0u8; GROUP_HEADER_SIZE];
    bytes[..24].copy_from_slice(b"RedWolf Design GrpFolder");
    bytes[28..32].copy_from_slice(&1i32.to_le_bytes()); // ver1
    bytes[32..36].copy_from_slice(&2i32.to_le_bytes()); // ver2
    bytes[36..40].copy_from_slice(&entry_count.to_le_bytes());
    scramble(&mut bytes);
    bytes
}

/// A 316-byte entry record naming `name` and a payload of `size` bytes.
fn entry(name: &str, size: i32, is_directory: bool) -> Vec<u8> {
    let mut bytes = vec![0u8; GROUP_ENTRY_SIZE];
    let name = name.as_bytes();
    bytes[..name.len()].copy_from_slice(name);
    bytes[264..268].copy_from_slice(&i32::from(is_directory).to_le_bytes());
    bytes[268..272].copy_from_slice(&size.to_le_bytes());
    bytes
}

fn open(data: Vec<u8>) -> Result<Group, clonk_resources::GroupError> {
    Group::from_memory(PathBuf::from("fuzz.c4g"), data)
}

/// A header is 204 bytes and every entry it promises costs another 316, so the
/// entry table cannot be larger than the input that carries it. Reserving from
/// the declared count instead lets a 204-byte file ask for an allocation of
/// `count * size_of::<PackedEntry>()` before a single entry is read — which is
/// an abort, not a rejection, because Rust's allocator failure path is not
/// recoverable.
#[test]
fn a_declared_entry_count_larger_than_the_input_is_rejected_not_reserved() {
    for declared in [i32::MAX, i32::MAX / 2, 1 << 24, 1 << 20] {
        let result = open(header(declared));
        assert!(
            result.is_err(),
            "a bare header declaring {declared} entries must fail, not allocate for them"
        );
    }
}

/// The same bound with one real entry present: the reader must still refuse the
/// rest rather than reserve for them.
#[test]
fn a_truncated_entry_table_fails_after_its_last_complete_record() {
    let mut data = header(i32::MAX);
    data.extend_from_slice(&entry("Script.c", 0, false));
    assert!(
        open(data).is_err(),
        "an entry table that stops early is a truncated group"
    );
}

/// Payload sizes are read from the entry records and summed to place each
/// entry, so a forged size must be caught when the bytes are read rather than
/// producing a buffer that the file never contained.
#[test]
fn a_forged_payload_size_cannot_read_past_the_group() {
    let mut data = header(1);
    data.extend_from_slice(&entry("Script.c", 1 << 30, false));
    data.extend_from_slice(b"short");
    let group = open(data).expect("the header and entry table are well formed");
    let entries = group.entries().expect("entry enumeration succeeds");
    assert_eq!(entries.len(), 1);
    for entry in &entries {
        // Typed error, never a panic or an out-of-bounds read.
        let _ = group.read_entry_bytes_exact(entry);
    }
}

/// Every entry's payload has to come from the group image, so the total number
/// of bytes the reader can hand back is bounded by the image itself.
#[test]
fn total_extracted_bytes_stay_within_the_group_image() {
    let payload = b"0123456789";
    let mut data = header(3);
    for name in ["A.txt", "B.txt", "C.txt"] {
        data.extend_from_slice(&entry(name, payload.len() as i32, false));
    }
    for _ in 0..3 {
        data.extend_from_slice(payload);
    }
    let image_len = data.len();

    let group = open(data).expect("a well-formed group opens");
    let entries = group.entries().expect("entry enumeration succeeds");
    let extracted: usize = entries
        .iter()
        .filter_map(|entry| group.read_entry_bytes_exact(entry).ok())
        .map(|bytes| bytes.len())
        .sum();
    assert!(
        extracted <= image_len,
        "extracted {extracted} bytes from a {image_len}-byte group"
    );
}

/// Duplicate and case-colliding names are a C4Group reality — the reader drops
/// the earlier record and marks the group for rewrite — so they must resolve to
/// one entry rather than corrupting the lookup index.
#[test]
fn duplicate_and_case_colliding_names_resolve_without_panicking() {
    let payload = b"xy";
    let mut data = header(3);
    for name in ["Same.txt", "SAME.TXT", "same.TxT"] {
        data.extend_from_slice(&entry(name, payload.len() as i32, false));
    }
    for _ in 0..3 {
        data.extend_from_slice(payload);
    }

    let group = open(data).expect("colliding names still open");
    let entries = group.entries().expect("entry enumeration succeeds");
    assert_eq!(
        entries.len(),
        1,
        "the collisions collapse to one live entry"
    );
    let _ = group.read_file("Same.txt");
    let _ = group.read_file("SAME.TXT");
}

/// Garbage, truncations and header edits all have to come back as typed errors.
#[test]
fn malformed_images_return_typed_errors() {
    let valid = {
        let mut data = header(1);
        data.extend_from_slice(&entry("Script.c", 2, false));
        data.extend_from_slice(b"ab");
        data
    };

    let mut cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"not a group at all".to_vec(),
        vec![0u8; GROUP_HEADER_SIZE],
        vec![0xffu8; GROUP_HEADER_SIZE + GROUP_ENTRY_SIZE],
    ];
    // Every truncation of a valid image.
    for len in 0..valid.len() {
        cases.push(valid[..len].to_vec());
    }
    // Every single-byte corruption of the header.
    for index in 0..GROUP_HEADER_SIZE {
        let mut corrupted = valid.clone();
        corrupted[index] ^= 0xff;
        cases.push(corrupted);
    }

    for case in cases {
        if let Ok(group) = open(case) {
            if let Ok(entries) = group.entries() {
                for entry in &entries {
                    let _ = group.read_entry_bytes_exact(entry);
                    let _ = group.open_child_entry_exact(entry);
                }
            }
        }
    }
}

/// A child group is an entry whose payload is itself a group image. Nesting is
/// therefore attacker-controlled depth, and opening one must not recurse
/// without bound.
#[test]
fn nested_child_groups_open_to_a_bounded_depth() {
    // Innermost group: one small entry.
    let mut image = {
        let mut data = header(1);
        data.extend_from_slice(&entry("Leaf.txt", 2, false));
        data.extend_from_slice(b"hi");
        data
    };

    for _ in 0..24 {
        let mut outer = header(1);
        outer.extend_from_slice(&entry("Inner.c4g", image.len() as i32, true));
        outer.extend_from_slice(&image);
        image = outer;
    }

    let mut group = open(image).expect("the outermost group opens");
    let mut depth = 0;
    while let Ok(child) = group.open_child("Inner.c4g") {
        depth += 1;
        assert!(depth <= 64, "child opening must terminate");
        group = child;
    }
    assert!(depth > 0, "the nesting was actually traversed");
}

/// The measured form of the bound above: opening a 204-byte header must not
/// reserve in proportion to the *declared* entry count. `i32::MAX` entries at
/// `size_of::<PackedEntry>()` each is hundreds of gigabytes, which a lazily
/// committing allocator will happily promise — so assert the number rather than
/// relying on the process to fall over.
#[test]
fn opening_a_header_reserves_in_proportion_to_the_input_not_the_declared_count() {
    let image = header(i32::MAX);
    let input_len = image.len();
    let (result, allocated) = allocated_during(|| open(image));
    assert!(result.is_err(), "a bare header is not a group");
    assert!(
        allocated < 1 << 20,
        "opening a {input_len}-byte header allocated {allocated} bytes for its \
         declared entry count"
    );
}
