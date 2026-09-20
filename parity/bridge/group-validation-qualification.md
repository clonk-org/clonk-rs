# Group bridge qualification

Qualified on 2026-09-20 UTC for clonk-org/clonk-rs#1265, on macOS 26.6.2
arm64 with Rust 1.98.1. The oracle remains pinned at
`7d43b47b7d789b533f32d005e64596e0a07019cd`; the builder applies the reviewed
observer patches described in [the bridge documentation](README.md#the-group-validation-bridge).

The current-tree build used `--with-group-validation --build-dir build-group`.
An external `CARGO_TARGET_DIR` was deliberately supplied; the builder correctly
overrode it with the directory imported by pinned CMake. The differential ran
with `--leaks`, against C++-written packed fixtures and ordinary folders.

| Fixture | Result |
| --- | --- |
| Folder, packed group, empty folder, empty packed group | No mismatch diagnostic; ABI values agree |
| Child folder and child packed group | No mismatch diagnostic; ABI values agree |
| Missing `Beta.txt` | Exactly one missing-entry diagnostic |
| Additional `Extra.txt` | Exactly one additional-entry diagnostic |
| Changed `Alpha.txt` size | Exactly one size diagnostic |
| Child group replaced by a file | Exactly one type diagnostic |

All ten fixture processes reported **0 leaks / 0 leaked bytes**. The ABI checks
compare entry order, names, file/directory types, sizes, existence, binary and
empty file buffers, and maker/root strings. They exercise all ten exported
functions and every allocation/free pair. Portable observer tests separately
exercise slash canonicalization and the permitted C++ argument evaluation
order that exposed the array-length ownership bug.

SHA-256 identities from the generated build record and differential result:

| Artifact | SHA-256 |
| --- | --- |
| Linked `liblc_resources.a` | `8b7d58e400b152ea6c295fc26fa8dcdc3dd06bac5398b58648a232a0045cdbc8` |
| Built oracle `clonk` | `92ad147dc23d748866e3a993355aaa8885936fa4ddefa97da064929e84a126ed` |
| Differential probe | `c56d0fc762e73c174e9f63a88345fd90bcd04fcb8ce6451d154cf70e5ca68c6a` |
| Resources `src/ffi.rs` | `7c74bc6c893ce16eff972a823fe3c0a52067a1f0c26259cd797fc2a988c27665` |
| Resources `src/group.rs` | `5ef5fff1915f2ce15ec26e770ee04d775523e10cd66fd037d6d98e28dd0b6fb3` |
| Pinned `lc_group_ffi.h` | `7e46a35646f875cec19254ab2daf7cf3205dcd7e34fc697508c62f4c69e86b3d` |

The generated record also binds 1,028 current-tree source files and 195 linked
artifacts/build inputs. The runner verifies it before and after the fixtures.
Rebuilding creates a new record; timestamps and paths can change binary hashes.
Use the documented commands to regenerate the per-fixture logs, probe link
command, build record and `result.json` in a fresh evidence directory.

The full-engine `run-group-differential.sh --leaks` also passed after integrating
clonk-org/clonk-rs#1648, including C++-packed and Rust-packed inputs, the real
system folder, and its read/open fault hooks. Its two leak checks retained the
three allocations from the oracle's `GetNonTranslocatedPath` CoreFoundation URL
creation (320 and 336 bytes); none reached the group bridge.

This qualification covers the group ABI and observer only. It does not assert
full-scenario simulation parity or qualify another operating system.
