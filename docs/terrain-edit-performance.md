# Small script terrain edits

Small script terrain edits now share untouched terrain and maintain warmed
column summaries from the actual pixel mutations. Callback previews retain
immediate read-after-write visibility and independent snapshots.

## Measurements

Final release samples and source/executable fingerprints are retained in
[`benchmarks/results/terrain-edits.json`](../benchmarks/results/terrain-edits.json).
The benchmark source is
[`terrain_edits.rs`](../crates/clonk-engine/benches/terrain_edits.rs).

The baseline is `a9fcad417a67833fd958753983a216e52678e1e0`. Both revisions
use the same benchmark source, lockfile, release profile, seed zero, and content
commit `0888b4f3bd10762c976c2fe93aa650d7909c1c6f`. Measurements run on an
AMD Ryzen 9 9900X under Linux with Rust 1.98.1. Each variant runs twice in
baseline/candidate/candidate/baseline order, one benchmark process at a time.
Every case creates 12 fresh engines per trial, discards three warmup samples,
and retains nine. Reported times pool all 18 retained samples per variant;
no outliers are removed. Other work on the host can affect these timings.

The fixture executes 200 native radius-two `DigFree` callbacks. Its effect
variant uses 200 real C4Script effect timers and times the complete
`tick_without_snapshot`, including callback dispatch, pixel reactions,
authoritative application, and the remaining simulation. It models the
callback fan-out of the shipped ClonkMars Terraformer: `CreateDigger` in
`content/ClonkMars.c4d/Structures.c4d/Terraformer.c4d/Script.c` creates up to
200 effects, and `FxDigEarthTimer` calls radius-two `DigFree`. This benchmark
is a controlled synthetic fixture, with simpler scripts and terrain than the
complete Terraformer scenario. Loading, engine setup, snapshot construction,
rendering, and explicit whole-plane hashing are outside the timed interval.
A retained-snapshot case holds the preceding snapshot across the timed edits.

A cold case performs its first batch of edits. A warm case first runs one batch
outside the timer, then edits the next band four pixels lower. The 1488×1536
size matches the existing Alchemy landscape fixture; 4096×4096 tests a large
world; 512×256 is the small-world control.

| Workload (200 callbacks) | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| 4096×4096, warm callbacks | 93.237 ms | 1.656 ms | 56.3× |
| 4096×4096, warm callbacks with retained snapshot | 100.733 ms | 1.817 ms | 55.4× |
| 4096×4096, warm complete effect tick | 114.763 ms | 18.543 ms | 6.2× |
| 4096×4096, warm complete effect tick with retained snapshot | 116.799 ms | 18.051 ms | 6.5× |
| 4096×4096, cold callbacks | 110.670 ms | 11.056 ms | 10.0× |
| 4096×4096, cold complete effect tick | 122.683 ms | 26.217 ms | 4.7× |
| 1488×1536, warm callbacks | 13.579 ms | 1.642 ms | 8.3× |
| 1488×1536, warm complete effect tick | 27.981 ms | 17.624 ms | 1.6× |
| 512×256, warm callbacks | 2.525 ms | 1.647 ms | 1.5× |
| 512×256, warm complete effect tick | 18.278 ms | 17.266 ms | 1.1× |

| Read control (one million queries) | Before | After | Added time |
| --- | ---: | ---: | ---: |
| Unchanged flat plane | 0.990 ms | 1.130 ms | +0.140 ms (+14%) |
| Shared plane with one changed pixel | 0.948 ms | 1.364 ms | +0.416 ms (+44%) |

The read controls expose the cost of checking for shared page overrides.
The block filter reduces the earlier exploratory shared-read result from
about 5.8 ms to the final 1.36 ms, but reads still cost more than indexing a
plain vector. The complete-tick measurements above include terrain queries;
they do not establish the cost in query-heavy scenarios with few edits.

All 18 terrain cases produce matching complete pixel-plane fingerprints and
`RandomCount` values between revisions. The retained snapshot still contains
the pre-edit pixels. Read controls perform one million deterministic
`byte_at` queries and compare their checksums.

This establishes a large speedup for terrain-edit-heavy callbacks, not a
universal frame-rate multiplier or measured performance on an older laptop.
Complete ticks also include effect dispatch and other simulation work, which
limits the resulting speedup. The small-world control has little tick benefit.

## Storage and derived state

- Uniquely owned arrays keep a flat allocation. Shared arrays detach 256-byte
  terrain/PixCnt pages or 32-entry column pages through a persistent tree.
  Untouched bytes, material lookup tables, retained raster inputs, and
  unchanged tunnel columns stay shared. Small root pointer tables still grow
  with the world, but callbacks no longer copy its pixel plane or every column.
- Reads outside changed blocks bypass the overlay through a bounded filter.
  The filter can have false positives; those use the ordinary checked tree
  lookup. It never changes the returned byte. Once the last baseline owner
  drops, only changed pages fold back into the original flat allocation. If a
  consumer already materialized an edited view, the next write reuses that
  allocation as its new baseline instead of discarding it.
- Column summaries initialize lazily and then update solid, liquid, and IFT
  spans in the existing pixel-write order. Removing a floating surface pixel
  jumps directly to the next solid span. Native writes to unused caches do
  not initialize them. Dig callbacks share their pre-edit column baseline
  with the authoritative replay.
- Explicit contiguous views materialize a shared edited array once; saves
  retain their original serialized representation. Polygon/chunk drawing, map remapping, and other bulk operations
  still use the full-plane path. Changing tunnel topology can still detach
  the tunnel map. First-use column scans and operations on highly fragmented
  span lists remain proportional to that column's data.

## Regression evidence

The allocation test calls a real script callback, checks its immediate
`GetMaterial` result and authoritative pixel, and compares a 512×256 world
with a 4096×4096 world. Before the change, the large callback allocated
17,031,167 bytes versus 273,755 bytes on the small world. The final callback
measurement is 128,586 bytes on the large world versus 131,358 bytes on
the small world: about 132× fewer allocated bytes for the large callback.
The regression allows at most 16 KiB of additional allocation as world size
grows.

Warm-edit regressions require zero full-height scans and zero surface probes
across a 1,500-pixel empty gap. Another test compares incremental summaries
with the original full-scan algorithm after 4,000 seeded, ordered solid,
liquid, IFT, and raw-mask mutations, then verifies snapshot isolation and
serialization. Storage tests exercise page, block, and branch boundaries,
filter collisions, cached contiguous views, and bulk resize.

The full workspace tests, clippy, engine snapshots, C++ primitive parity,
compatibility-profile verification, formatting, Python script tests, xtask
engine-tool tests, and change-aware replay/render checks are required before
landing. Primitive parity and these fixture fingerprints do not establish
full-scenario C++ parity.

Reproduce the component measurements with:

```sh
cargo bench -p clonk-engine --bench terrain_edits
```

For a baseline comparison, copy only this benchmark and its `[[bench]]`
registration into an isolated checkout of the baseline commit; keep each
checkout's target directory independent and finish compilation before timing.
