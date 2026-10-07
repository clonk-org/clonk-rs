# Callback snapshot sharing

This is a preparatory refactor for clonk-org/clonk-rs#1864. The issue remains
open: callbacks still stage and replay mutations, context objects are still
built per callback, and both performance acceptance targets remain unmet.

## Changes

Receiver snapshots share the backing storage for effects, vertices, component
order, graphics overlays and ordered component entries. Mutable access detaches
the buffer, preserving independent snapshot values. Savegame serialization and
public presentation snapshots retain their existing representation and order.

Crew-info rosters are projected on their first host read or write. Callback-world
clones share one mutable projection, so nested crew operations retain their
immediate visibility. Separate callbacks have independent projections. The raw
projector borrows only frozen roster fields and the definition table under the
existing synchronous paused-engine lifetime contract.

## Measurements

Measured October 7, 2026, on AMD Ryzen 9 9900X, Linux 7.2.9-arch1-1,
Rust 1.98.1. Baseline: `c4f9336cd5d55a9f84c6b070519f79e87030f414`. Candidate code:
`b71c4b1fade6f142ec248e2368d20adfcf8a0c24`. Both probes use identical source,
Cargo.lock and release settings (`lto=thin`, one codegen unit, line tables).
Content: `122bda2e310ca7200c8d834cf1dbefc4ec6875fa` from the main checkout.
This differs from the test worktree's pinned content revision, recorded in the
raw artifact. Both builds load the same content and System scripts.

Three fresh-process pairs per scenario alternate execution order. After all
builds and validation finish, each process advances 3,200 frames sequentially;
frames 201–3,200 supply the reported timings. The table uses the median of
three run means and the median of each run's tail percentiles. No samples are
removed. These are simulation advance timings, excluding loading, snapshot
projection and rendering, and do not establish a frame-rate improvement.

| Scenario | Before ms/tick | After ms/tick | Reduction | p95 before / after ms | p99 before / after ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4_TowerOfMagic | 1.866 | 1.771 | 5.1% | 2.722 / 2.620 | 3.241 / 3.156 |
| SevenKeys | 1.165 | 1.112 | 4.5% | 1.657 / 1.603 | 3.357 / 3.217 |
| SkyBridge | 0.075 | 0.073 | 2.7% | 0.215 / 0.215 | 0.303 / 0.299 |

In separate 1,500-tick allocation runs, Tower of Magic falls from **20,866 to
16,269 allocations/tick (22.0%)**, and from **3,471,147 to 3,224,154 bytes/tick
(7.1%)**. These cover simulation advance only. They already exceed the issue's
1,000-allocation full app-update target; the 0.5 ms/tick timing target is also
unmet.

The strengthened warm-callback budget was observed RED with the receiver's deep
copy restored: **43 allocations without effects versus 236 with 64 effects**.
With shared snapshots it passes a budget of at most two additional allocations.
Other regression checks pin write isolation, unchanged Debug/serde vector output,
ordered duplicate components, lazy crew reads and shared nested crew edits.

## Behavior validation

Fresh baseline and candidate runs compare all serialized EngineState fields plus
the nonserialized landscape ScanX cursor at frames 1, 100, 300, 600, 1,000 and
1,500 across nine real scenarios. All **54 checkpoint pairs match**, including
raw fixed-point object state, synchronized RNG state and ordered lists. Only
JSON object-key ordering is ignored. Archived Debug hashes differ for Chaos;
fresh saved-state comparisons, ScanX and synchronization checks match there.
Debug exposes nonserialized runtime caches, so the old hash archive is retained
as a diagnostic rather than treated as an authoritative simulation oracle.
These comparisons do not prove full-scenario C++ parity.

Passed: 12,960 workspace tests (29 existing opt-ins skipped), clippy with warnings
denied, engine snapshots, primitive C++ parity, compatibility verification,
formatting, 1,038 Python tests (three skipped), 132 engine-tool tests (one ignored)
and all 12 dev-check steps. Compatibility verification was repeated after the
code commit because authoritative acquisition rejects uncommitted source drift.

## Reproduction

[Raw samples, fingerprints, harness and runner](../benchmarks/results/callback-snapshot-sharing.json)
include every frame duration, per-run summaries, all checkpoint hashes, source
hashes, binary hashes and the legacy Debug digest archive. The harness records
allocations with the system allocator and counts only advance work. Its
STATE_PREFIX mode captures full saved state plus ScanX outside the timed region.

Extract the baseline source from the recorded commit and compile the embedded
harness separately against baseline and candidate in independent target
directories. Set its workspace/content roots to the recorded inputs (or equivalent
local paths) before running the embedded runner. Do not share target directories.
