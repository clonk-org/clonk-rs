# LegacyClonk C++ → Rust Port Plan

**Updated:** 2025-10-26 | **Goal:** Run ALL real scenarios with exact C++ parity | **Status:** ⛔ BLOCKED

---

## TL;DR

- **P0 BLOCKERS:** (1) AI command system missing (2) Engine smoke tests failing (build/clear/objective parity)
- **Scenario Discovery:** ✅ Official content repo wired; menu now lists full scenario catalog
- **Current State:** Build compiles; targeted tests (`lc-platform`, `lc-app`) green; full workspace still red on legacy engine cases
- **Next 48h:** Stabilize engine tests; sketch AI command trait; triage panics
- **Exit Criteria:** P0 cleared + CI green + smoke tests pass

---

## P0 BLOCKERS (Nothing works without these)

### 0. Scenario Discovery — ✅ DONE
**Fix:** Vendor official `legacyclonk/content` repo as git submodule and extend `AppPaths` to detect an optional `content/` root (incl. env override) when assembling scenario/material/object search paths.
**Verification:** `cargo test -p lc-app load_frontend_scenarios_discovers_repository_content` + manual run show ≥5 scenarios; fallback sandbox no longer sole entry.
**Follow-up:** Document release packaging (ensure content submodule pulled in CI artifacts) and add smoke run once engine blockers cleared.

---

### 1. AI Command System — MISSING ⛔
**Problem:** Zero AI commands ported; no `AddCommand`/`SetCommand` host functions
**Evidence:** `grep -r "AddCommand\|SetCommand" rust/crates/lc-engine/src/compat.rs` → 0 results
**C++ Has:** 30 commands (Follow, MoveTo, Build, Attack, etc.) via `C4Command.cpp`
**Rust Has:** Nothing
**Impact:** Buildings can't request materials; crew have no AI; production broken; animals static
**Next:**
1. Extract C++ `C4Command` interface (signatures, lifecycle, queue mechanics)
2. Design Rust command trait + queue in `lc-engine/src/command.rs`
3. Implement 2 exemplar commands (MoveTo, Build) with acceptance tests
4. Wire into object update loop

**Accept:** `AddCommand("MoveTo", ...)` works in script; crew moves to target; command queue processes
**Owner:** TBD | **ETA:** Weeks 2-4 | **Risk:** HIGH (large system, hidden deps)

---

### 2. Test Suite — BROKEN ⛔
**Problem:** Workspace tests compile again, but parity checks still fail (construction components, legacy objectives, missing host functions).
**Evidence:**
```bash
cargo test --workspace
  compat::tests::host_function_registration_matches_expected
  tests::build_procedure_requires_components_before_progress
  tests::build_procedure_consumes_components_from_builder
  tests::legacy_clear_object_objective_triggers_after_removal
  tests::script_game_over_triggers_on_game_over
```
**Impact:** Cannot claim scenario parity or CI green while these regressions remain.
**Next:**
1. Restore host API coverage once AI command trait exists.
2. Port construction component bookkeeping (`C4ObjectCom`) to satisfy build tests.
3. Implement legacy objective triggers / `PreInitializePlayer`.
4. Gate workspace tests in CI after failures addressed.

**Accept:** `cargo test --workspace` passes (no fails) locally + CI
**Owner:** TBD | **ETA:** Week 1 | **Risk:** MEDIUM (breadth)

---

## P1 CRITICAL (Breaks many scenarios)

### 3. Production Code Instability
**Problem:** 45 `panic!()`/`todo!()`/`unimplemented!()` in lc-engine
**Evidence:** `rg -n "panic!\(|todo!\(|unimplemented!\(" rust/crates/lc-engine/src | wc -l` → 45
**Next:** Categorize (error path vs unreachable) → convert to `Result` or document exceptions
**Accept:** 0 panic!/todo! in non-test production code
**Owner:** TBD | **ETA:** Week 5

### 4. Phase Actions — MISSING
**Problem:** No PhaseAction system (climbing, complex animations)
**Next:** Port from C++ `PhaseAction` logic
**Accept:** Objects transition through phase actions correctly
**Owner:** TBD | **ETA:** Week 6

### 5. Particle System — INCOMPLETE
**Problem:** Only `CreateParticle` exists; no def loading (C4ParticleDef, custom behaviors)
**Next:** Port particle def loading + proc callbacks
**Accept:** Custom particle FX from Particle.txt work
**Owner:** TBD | **ETA:** Week 7

---

## STATUS MATRIX (Single Source of Truth)

| System | Status | Evidence | Owner |
|--------|--------|----------|-------|
| **Build/Compile** | ✅ Working | `cargo build --release` succeeds | - |
| **Tests** | ⛔ Broken | Workspace fail: host functions + construction/objective tests | TBD |
| **Scenario Discovery** | ✅ Working | content/ assets discovered; sandbox no longer sole entry | TBD |
| **Scenario Loading** | ⚠️ Unknown | Needs real scenarios to test | TBD |
| **AI Commands** | ⛔ Missing | No AddCommand/SetCommand | TBD |
| **Graphics Rendering** | ✅ Implemented | Code exists; needs verification | TBD |
| **Player Input** | ✅ Implemented | Keyboard/mouse/gamepad code exists | TBD |
| **Landscape** | ✅ Implemented | Dig/blast/materials code exists | TBD |
| **Audio** | ✅ Implemented | Music/SFX code exists | TBD |
| **Multiplayer** | ⚠️ Unknown | Transport exists; needs testing | TBD |
| **Save/Load** | ✅ Implemented | Quick save/load code exists | TBD |
| **Scripting** | ⚠️ Partial | 121 host functions; missing commands | TBD |
| **Menus/HUD** | ✅ Implemented | Code exists; needs verification | TBD |
| **Weather** | ✅ Implemented | Code exists; needs verification | TBD |
| **CI** | ⛔ Presumed Red | Tests don't compile | TBD |

**Legend:** ✅ Working | ⚠️ Unknown/Partial | ⛔ Blocked/Missing

---

## VERIFIED WORKING (Code Inspection)

✅ **Core Engine:** 121 C4Aul host functions registered
✅ **Architecture:** Clean separation (15 crates), well-organized
✅ **Main Menu:** UI exists (`startup_main_menu.rs`) with correct buttons
✅ **Scenario Parser:** Scenario.txt, landscape, Objects.txt loading implemented
✅ **Fallback Design:** Sandbox appears only when discovery fails (good design)

---

## 48H ACTION PLAN

**Day 1: ENGINE TEST TRIAGE**
- [ ] Summarize failing engine tests + map missing C++ subsystems (commands, construction, objectives)
- [ ] Draft plan to port construction component bookkeeping (C4ObjectCom) and associated tests
- [ ] Identify minimal `PreInitializePlayer`/objective hooks needed for parity smoke test

**Day 2: STUB AI COMMANDS**
- [ ] Extract C++ AddCommand/SetCommand signatures from `C4Command.cpp`
- [ ] Create `lc-engine/src/command.rs` skeleton
- [ ] Wire stub into host function registry (returns "not implemented" for now)
- [ ] Document AI command implementation plan

**Ongoing:**
- [x] Fix test schema drift → get `cargo test` compiling
- [ ] Audit top 10 panic!/todo! by likelihood → convert to Result
- [ ] Create verification matrix for each system

---

## QUALITY GATES (Definition of Done)

- ✅ P0 blockers cleared
- ✅ CI green (build + tests compile + smokes pass)
- ✅ Scenario list shows real scenarios
- ✅ 0 panic!/todo! in production src
- ✅ Each system has ≥1 passing smoke test

---

## EVIDENCE & COMMANDS

**Scenario Discovery:**
```bash
cargo test -p lc-app load_frontend_scenarios_discovers_repository_content
```

**Test Status:**
```bash
cargo test --workspace  # currently fails on engine parity cases
```

**Panic Count:**
```bash
rg -n "panic!\(|todo!\(|unimplemented!\(" rust/crates/lc-engine/src | wc -l  # 45
```

**Host Functions:**
```bash
rg "script.register_host_function" rust/crates/lc-engine/src/compat.rs | wc -l  # 121
```

---

## OPEN QUESTIONS

1. **Content packaging:** How do we bundle/ship the `content/` submodule for CI + releases?
2. **Smoke coverage:** Which scenarios should anchor first parity runs now discovery is live?
3. **C++ AddCommand full contract?** (threading, lifecycle, queue mechanics)
4. **Phase action priority?** (Required for which scenarios?)

---

## RISKS & MITIGATION

| Risk | Impact | Mitigation |
|------|--------|------------|
| Hidden AI command deps | HIGH | Extract full C++ contract before implementing |
| Schema drift continues | MEDIUM | CI gate: tests must compile before merge |
| Unknown system failures | MEDIUM | Create smoke test per system; verify in CI |
| Long build times (5+ min) | LOW | Investigate dep tree; check build cache |

---

## NEXT OWNER ACTIONS

**Immediate (This Week):**
1. Install scenarios → verify discovery works
2. Fix test compilation
3. Document AI command implementation plan
4. Triage panic!/todo! → file issues

**Short Term (Weeks 2-4):**
1. Implement AI command system
2. Verify all systems with smoke tests
3. Get CI green

**Medium Term (Weeks 5-8):**
1. Complete phase actions
2. Complete particle system
3. Run full parity validation

---

## APPENDIX: FILE REFERENCES

**Key Files:**
- Scenario discovery: `rust/crates/lc-app/src/main.rs:8319` (`load_frontend_scenarios`)
- Command system (C++): `src/C4Command.cpp`, `src/C4Command.h`
- Host functions: `rust/crates/lc-engine/src/compat.rs:1848` (`register_host_functions`)
- Test failures: `rust/crates/lc-engine/tests/*`, `rust/crates/lc-app/src/main.rs` (test module)
- Main menu: `rust/crates/lc-frontend/src/startup_main_menu.rs`

**C++ Reference:**
- Command types: `src/C4Command.h:27-57` (30 commands defined)
- Command execution: `src/C4Command.cpp` (evaluation logic)
- Particle system: `src/C4Particles.h`, `src/C4Particles.cpp`
