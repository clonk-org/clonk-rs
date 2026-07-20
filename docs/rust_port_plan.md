# LegacyClonk C++ → Rust Port Plan

**Updated:** 2025-10-27 | **Goal:** Run ALL real scenarios with exact C++ parity | **Status:** ⛔ BLOCKED

---

## TL;DR

- **P0 BLOCKERS:** AI command system parity on Acquire/Buy snapshots + AddCommand lifecycle now implemented; remaining work is broader command heuristics audit
- **Scenario Discovery:** ✅ Official content repo wired; menu now lists full scenario catalog
- **Current State:** Build compiles; workspace tests green; Build/Construct/Exit/Grab/UnGrab/Throw/Wait/Jump/Chop/Get/Sell/Context/Take/Take2 commands implemented; Acquire/Buy snapshots + AddCommand retries now mirror C++
- **Next 48h:** Keep engine tests green; audit remaining command heuristics (multi-crew/multi-target); triage panics
- **Exit Criteria:** P0 cleared + CI green + smoke tests pass

---

## P0 BLOCKERS (Nothing works without these)

### 0. Scenario Discovery — ✅ DONE
**Fix:** Vendor official `legacyclonk/content` repo as git submodule and extend `AppPaths` to detect an optional `content/` root (incl. env override) when assembling scenario/material/object search paths.
**Verification:** `cargo test -p lc-app load_frontend_scenarios_discovers_repository_content` + manual run show ≥5 scenarios; fallback sandbox no longer sole entry.
**Follow-up:** Document release packaging (ensure content submodule pulled in CI artifacts) and add smoke run once engine blockers cleared.

---

### 1. AI Command System — PARTIAL ⚠️
**Problem:** Command stack covers MoveTo/Follow/Enter/Exit/Build/Attack/Acquire/Buy/Energy + Grab/UnGrab/Throw/Wait/Jump/Chop/Dig/Home/Put/Drop/Transfer; Acquire/Buy snapshot parity + AddCommand lifecycle now align with C++, but we still need broader multi-command heuristics validation; Sell mirrors base transactions via Rust command state and tests
**Progress:** Acquire now queues `Get` subcommands (and reuses existing container/exit heuristics), command stack snapshots persist Acquire/Buy state, and base commands track failures/retries like `C4Command::Execute`; Take/Take2 remain ported; AI commands raise inventory/get menu requests with parity tests and UI container mode support
**Evidence:** `command::tests::acquire_requests_get_for_candidate`, `command::tests::acquire_requests_get_when_in_other_container`, and `command_stack_snapshot_preserves_acquire_state` exercise new Acquire flow; `command_stack_snapshot_preserves_buy_state` covers Buy persistence; `command::tests::failing_subcommand_increments_base_failures_and_schedules_retry` verifies AddCommand retry propagation; prior parity suites (`command::tests::exit_moves_into_parent_container_when_nested`, `command::tests::grab_starts_push_when_in_range`, `command::tests::wait_stops_dig_and_completes_after_interval`, `command::tests::chop_sets_action_when_in_range`, `command::tests::jump_sets_direction_and_action_when_walking`, `command::tests::home_requests_enter_when_not_in_base`, `command::tests::push_to_completes_with_wait_and_ungrab_when_in_position`, `command::tests::context_emits_menu_request`, `command::tests::take_opens_activate_menu`) continue to pass; workspace: `cargo test --workspace`
**C++ Has:** 30 commands (Follow, MoveTo, Build, Attack, etc.) via `C4Command.cpp`
**Rust Has:** Command trait infrastructure + MoveTo/Follow/Enter/Exit/Build/Attack/Throw/Chop/Wait/Jump/Dig/Get/Home/Put/Drop/Construct/Activate/PushTo + Retry countdown for failure back-off; Acquire now delegates through Get for loose items, shared-container withdrawal, and cross-container exit heuristics; Buy commands deduct home base stock/wealth, spawn purchased items in bases, and hand off targeted shop purchases; Activate mirrors container activation (enter/exit + owner handoff) with tests `command::tests::activate_completes_when_target_outside_container`, `command::tests::activate_requests_enter_when_actor_outside_container`, and `command::tests::activate_releases_target_when_inside_container`; PushTo queues Activate/Grab/Enter/MoveTo/Wait/UnGrab in the C++ order and guards build/dig cancellation (see `command::tests::push_to_completes_with_wait_and_ungrab_when_in_position`); Get moves targeted pickups from ground and containers (exit/enter/grab heuristics, dig-out fallback) with parity tests; Put transfers inventory into structures and requests Exit/Move/UnGrab when needed; Drop covers targeted ground releases and delegates to Put for container flows (see `command::tests::put_transfers_item_into_target_container`, `command::tests::drop_transfers_item_to_ground`); Construct consumes conkits, spawns construction sites, and queues Build parity (see `command::tests::construct_spawns_construction_and_queues_build`); Energy line-kit handshake; Grab/UnGrab mirror C++ push start/stop (auto queues UnGrab when switching targets); Home seeks the nearest friendly base and queues Enter until crew arrive; Chop respects approach heuristics, ungrabs push targets, and starts forced Chop action; Dig mirrors C++ flow (auto-ungrab, exit containers, forces dig action, enforces `Dig2Object` flag, stops at destination); Jump aligns facing and injects Jump action when walkers command it; Call dispatches scripted callbacks via `command::tests::call_emits_event_and_completes`; Context pushes frontend menu requests captured in simulation snapshots; command stack snapshots persist across engine saves and scenario exports
**Impact:** Build automation works (crew picks up components, honours material requirements); Exit mirrors C++ nested-container behaviour and stops builders before ejecting; Grab reproduces push engagement and respects hang/build dig-outs; PushTo preserves the C++ Activate→Grab→Enter/MoveTo→Wait/UnGrab queue so pushed items reach structures/positions reliably; Throw handles targeted throws with Acquire fallback; Get delivers ground/container pickups and queues Exit/UnGrab as needed; Put hands contents into bases with matching heuristics; Drop releases to ground or delegates to Put while respecting push/containment; Jump applies C++-style orientation + action forcing; Energize follows loose line-kit pickups; merchant/shop buys succeed and survive save/load; Context commands now signal frontend auto-open; container/inventory menus now scriptable via Take/Take2; remaining gaps: multi-command heuristic audit + scenario validation
**Next:**
1. ✅ Expand tests to cover queue chaining, failure recovery, and scenario parity (new `command_stack_snapshot_preserves_acquire_state` ordering assertions, `acquire_retries_buy_after_cooldown`, `command_stack_put_transfers_item_into_container`)
2. Audit Acquire/Buy persistence + AddCommand lifecycle against C++ (save/load + queue heuristics)

**Accept:** `AddCommand("MoveTo", ...)` and `SetCommand("Build", ...)` reproduce C++ behaviour across parity scenarios
**Owner:** TBD | **ETA:** Weeks 2-4 | **Risk:** HIGH (large system, hidden deps)

---

### 2. Test Suite — ✅
**Fix:** Delay objective polling until after the first simulation tick and make scenario callbacks tolerant of missing `PreInitializePlayer`.
**Verification:** `cargo test --workspace`

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
| **Tests** | ✅ Working | `cargo test --workspace` passes | TBD |
| **Scenario Discovery** | ✅ Working | content/ assets discovered; sandbox no longer sole entry | TBD |
| **Scenario Loading** | ⚠️ Unknown | Needs real scenarios to test | TBD |
| **AI Commands** | ⚠️ Partial | MoveTo/Follow/Enter/Exit/Build/Attack/Wait/Retry/Throw/Chop/Jump/Home + Acquire + Grab/UnGrab handle loose/container/cross-container pickups; Buy spawns base purchases and targeted hand-offs; Activate covers container release parity; Take/Take2 now open inventory/get menus with UI support; PushTo mirrors C++ queue chaining with parity tests; Transfer mirrors zone entry + ControlTransfer script hand-off; outstanding: Acquire/Buy persistence + AddCommand lifecycle audit | TBD |
| **Graphics Rendering** | ✅ Implemented | Code exists; needs verification | TBD |
| **Player Input** | ✅ Implemented | Keyboard/mouse/gamepad code exists | TBD |
| **Landscape** | ✅ Implemented | Dig/blast/materials code exists | TBD |
| **Audio** | ✅ Implemented | Music/SFX code exists | TBD |
| **Multiplayer** | ⚠️ Unknown | Transport exists; needs testing | TBD |
| **Save/Load** | ✅ Implemented | Quick save/load code exists | TBD |
| **Scripting** | ⚠️ Partial | 121 host functions; missing commands | TBD |
| **Menus/HUD** | ✅ Implemented | Code exists; needs verification | TBD |
| **Weather** | ✅ Implemented | Code exists; needs verification | TBD |
| **CI** | ⛔ Presumed Red | Pipeline still disabled pending AI parity | TBD |

**Legend:** ✅ Working | ⚠️ Unknown/Partial | ⛔ Blocked/Missing

---

## VERIFIED WORKING (Code Inspection)

✅ **Core Engine:** 121 C4Aul host functions registered
✅ **Architecture:** Clean separation (15 crates), well-organized
✅ **Main Menu:** UI exists (`startup_main_menu.rs`) with correct buttons
✅ **Scenario Parser:** Scenario.txt, landscape, Objects.txt loading implemented
✅ **Fallback Design:** Sandbox appears only when discovery fails (good design)
✅ **AI Build Command:** Rust `SetCommand("Build")` starts construction, respects component requirements
✅ **AI Follow/Attack:** Command stack issues MoveTo + direction updates mirroring baseline C4 behaviour (unit tested)
✅ **AI Acquire/Buy:** Shared/container/cross-container pulls mirrored via command events; base + targeted shop purchases adjust wealth/spawn objects (`command::tests::buy_spawns_item_and_updates_player_state`, `command::tests::buy_collects_item_from_explicit_target`)
✅ **AI Grab/UnGrab:** Push engagement + release parity covered by unit tests (`command::tests::grab_starts_push_when_in_range`, `command::tests::ungrab_sets_idle_and_completes`)
✅ **AI Enter Command:** Crew enter structures when in range; MoveTo fallback covers distance (`command::tests::enter_enters_target_when_in_range`, `command::tests::enter_requests_move_when_far`)
✅ **AI Retry Cooldown:** Retry command mirrors C++ UpdateInterval back-off (`command::tests::retry_command_waits_then_completes`)
✅ **Command Persistence:** Command stack state captured and restored across saves (`command::tests::command_stack_snapshot_preserves_acquire_state`)

---

## 48H ACTION PLAN

**Day 1: ENGINE TEST TRIAGE**
- [ ] Summarize failing engine tests + map missing C++ subsystems (commands, construction, objectives)
- [ ] Draft plan to port construction component bookkeeping (C4ObjectCom) and associated tests
- [x] Identify minimal `PreInitializePlayer`/objective hooks needed for parity smoke test

**Day 2: STUB AI COMMANDS**
- [ ] Extract C++ AddCommand/SetCommand signatures from `C4Command.cpp`
- [x] Create `lc-engine/src/command.rs` skeleton
- [x] Wire host functions into registry and return real MoveTo behaviour
- [x] Document AI command implementation plan

**Ongoing:**
- [x] Fix test schema drift → get `cargo test` compiling
- [ ] Audit top 10 panic!/todo! by likelihood → convert to Result
- [ ] Create verification matrix for each system

---

## QUALITY GATES (Definition of Done)

- ⛔ P0 blockers cleared
- ⚠️ CI green (build + tests compile + smokes pass)
- ✅ Scenario list shows real scenarios
- ⛔ 0 panic!/todo! in production src
- ⚠️ Each system has ≥1 passing smoke test

---

## EVIDENCE & COMMANDS

**Scenario Discovery:**
```bash
cargo nextest run -p lc-app load_frontend_scenarios_discovers_repository_content
```

**Test Status:**
```bash
cargo nextest run --workspace  # passes
```

**Panic Count:**
```bash
rg -n "panic!\(|todo!\(|unimplemented!\(" rust/crates/lc-engine/src | wc -l  # 45
```

**Host Functions:**
```bash
rg "script.register_host_function" rust/crates/lc-engine/src/compat.rs | wc -l  # 121
```

**AI Commands:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::acquire_transfers_item_from_shared_container \
  command::tests::acquire_enters_container_when_adjacent \
  command::tests::buy_spawns_item_and_updates_player_state \
  command::tests::buy_collects_item_from_explicit_target \
  command::tests::command_stack_snapshot_preserves_acquire_state \
  command::tests::acquire_retries_buy_after_cooldown \
  command::tests::command_stack_put_transfers_item_into_container \
  command::tests::get_transfers_item_when_in_range \
  command::tests::get_requests_exit_when_actor_contained \
  command::tests::get_requests_ungrab_when_pushing_other_target
```

**Sell Command:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::sell_requires_definition \
  command::tests::sell_requests_enter_when_outside \
  command::tests::sell_completes_when_inside \
  command::tests::sell_fails_when_disabled
```

**Chop Command:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::chop_sets_action_when_in_range \
  command::tests::chop_requests_move_when_far \
  command::tests::chop_requests_ungrab_when_pushing \
  command::tests::chop_fails_when_builder_cannot_chop
```

**Throw Command:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::throw_requests_acquire_when_item_missing \
  command::tests::throw_pushes_move_to_target_when_out_of_range \
  command::tests::throw_sets_throw_action_when_in_range \
  command::tests::throw_requests_ungrab_when_pushing
```

**Dig Command:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::dig_requests_ungrab_when_pushing \
  command::tests::dig_requests_exit_when_contained \
  command::tests::dig_sets_dig_action_when_walking \
  command::tests::dig_completes_when_within_move_range
```

**PushTo Command:**
```bash
cargo nextest run -p lc-engine-unit-tests --test engine_inline \
  command::tests::push_to_requests_grab_when_actor_not_pushing \
  command::tests::push_to_requests_enter_when_destination_requires_container \
  command::tests::push_to_completes_with_wait_and_ungrab_when_in_position
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
