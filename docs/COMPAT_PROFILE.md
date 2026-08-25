# LegacyClonk compatibility profile

> The contract stated here is machine-readable in `compat/profile.json` and
> checked by `cargo xtask compat verify`. This document is the human half of the
> same contract: where the two disagree, they are both wrong and the change that
> separated them is incomplete. The oracle is the instrumented fork snapshot at
> `7d43b47b7d789b533f32d005e64596e0a07019cd`, not upstream LegacyClonk.

## What the profile is

The compatibility profile (`legacy-clonk`) is a named operating mode that says
what the port promises to reproduce from the pinned C++ engine, and — just as
importantly — what it does not. It exists so a "legacy mode" control can never
be a checkbox with an undefined meaning: the promise is written down, the
evidence for each part of it is named separately, every deliberate difference
has an owner and a disposition, and every combination the promise does not cover
is refused rather than approximated.

The profile is defined before it is implemented, on purpose. A configuration
key that turns on an undefined promise is worse than no key at all: it invites a
player into a mixed session that may desync, and a desync in a lockstep engine
is a lost round for everyone in it, not a cosmetic glitch. Implementation of the
toggle, its negotiation, and its enforcement are tracked separately in
clonk-org/clonk-rs#583, clonk-org/clonk-rs#584 and clonk-org/clonk-rs#588; the
tracker for the whole effort is clonk-org/clonk-rs#498.

## What the profile pins

| Pin | Value | Why it is pinned |
| --- | --- | --- |
| Engine version | `4.9.11.0 [362]` | The protocol identity and the content gate. `C4Network2` admission compares only the build against `C4XVERBUILD`, and definition gating prunes content declaring a newer engine. Single source: `crates/clonk-core/src/version.rs`. |
| Oracle commit | `7d43b47b7d789b533f32d005e64596e0a07019cd` | The instrumented fork snapshot every differential golden and every `file:line` citation in this contract refers to. Upstream LegacyClonk lacks the RNG-trace hooks and extracted headers the golden generator needs. |
| Content commit | the `content` submodule gitlink | The profile promises the behaviour of *this* content. A content bump changes what scripts do, so it must restate the pin here in the same change. |

`cargo xtask compat verify` cross-checks all three: the engine pin against
`clonk_core::version` on all of the tuple, the build, and the rendered text; the
oracle pin against the constant in the verifier; and the content pin against
`git ls-tree HEAD content`. Content drift is a gate failure, never a warning
(`fc-content-drift`).

## How to read the manifest

`compat/profile.json` carries schema `clonk-rs/compat-profile/v1` and five parts.

**Areas.** Every promise section, divergence, and port-only feature names one of
six contract areas: `simulation`, `control`, `transport`, `content`,
`presentation`, `save_replay`. The split is the point — evidence for the
simulation promise says nothing about the transport promise, and a single
"parity" claim covering all six would be unfalsifiable.

**Evidence.** Each area states its promise once and then lists evidence
separately. Evidence is a `command`, `test`, `document`, or `issue`, and is
either `held` (the check exists and passes today) or `pending`. Pending evidence
must cite a qualified `clonk-org/clonk-rs#N` reference, so a gap always names
the work that closes it.

**Dispositions.** A divergence is either `accepted` — a difference from C++ that
is understood, deliberate, and permanent — or an `open-gap`, which is a defect
that has not been fixed yet. The two are not interchangeable, and the verifier
enforces the distinction: an `open-gap` must carry `profile_action: blocked`,
and an `accepted` divergence may never be `blocked`.

**Profile actions.** `reverted` means the profile restores the C++ behaviour;
`kept` means the difference stands even inside the profile, because it cannot
change what a C++ peer would compute; `blocked` means the profile may not be
advertised while the entry is open.

**Owners.** Every divergence and port-only feature names where its disposition
was decided — an issue, `parity/README.md`, or a section of this document. A
divergence with no owner is an observation, not a decision, and fails
verification. Issue references are always written `clonk-org/clonk-rs#N`: this
repository is public and a bare `#N` is ambiguous against
`legacyclonk/LegacyClonk`.

Anything determinism-critical must also name the C++ it differs from
(`cpp_reference`); a sync-relevant claim that never locates itself in the oracle
cannot be checked by a reader.

## The promise, area by area

### Simulation

With a fixed seed and a fixed frame-counted control sequence, engine state is
bit-identical to the oracle in every raw `C4Fixed` value, the
`Random`/`RandomCount`/`FRnd3` ledgers, landscape and material state, PXS
execution, and C4Script VM semantics — except at the two accepted safety
boundaries below. No intentional simulation divergence is retained by the
profile.

Held: `cargo xtask parity verify` (the C++-golden primitive sections) and
`cargo xtask engine-snapshots verify` (Rust self-consistency of the synthetic
scenarios). `parity/reports/goldrush_seed_424242.json` is a historical
continuous shadow differential and is scoped evidence about its own bundled
revision only.

Pending: clonk-org/clonk-rs#585, a current-tree full-scenario C++ shadow diff.
`cargo xtask parity verify` is roughly 31 primitive sections; it passes untouched
through a change to players, savegames, or scenario init, so it is not proof of
full-scenario parity and this contract does not present it as such.

### Control

Control events reach the engine with C++ semantics. AutoStop key routing is
exactly `C4Game::LocalControlKeyUp`, and inside the profile the classic-set
key-up release is suppressed exactly as C++ suppresses it. Host-side latency
pacing selects no different synchronized events.

Held: the classic key-up lockstep tests in `crates/clonk-app/src/main_tests.rs`,
and the PreSend and latency-budget tests in
`crates/clonk-app-netplay/src/network.rs`, which keep the ACT rolling average
and its 1..15 clamp bit-exact with C++.

Pending: clonk-org/clonk-rs#586, real stock-C++/Rust host-client interoperation
in both directions.

### Transport

The port speaks the pinned C++ wire protocol: `C4Network` PID admission with the
build check, ReliableUDP v2, and NetPuncher v1. The transport-quality
divergences are host-local or browser-local and change no synchronized data.
Mixed C++/Rust sessions are refused until interoperation is actually proven —
the wire format matching is necessary, not sufficient.

Held: the ReliableUDP, NetPuncher, and session-protocol conformance tests in
`crates/clonk-network`, including the `C4Network2.cpp:1291-1299` build
comparison; and `crates/clonk-network/src/capabilities.rs`, which pins what a
released port build reads from this build's announcement and datagrams.

Pending: clonk-org/clonk-rs#583 (advertising and enforcing the profile during
connection setup) and clonk-org/clonk-rs#586.

### Content and resources

Bundled content is exactly the pinned content commit, and the eight
port-authored `planet/System.c4g` `#appendto` scripts are disabled, so
definitions and scripts behave exactly as the shipped content specifies.
Definition version gating prunes content newer than the engine exactly as C++
does.

Held: `cargo xtask scenario-sweep` loads and applies every real scenario under
`content/` with real definitions and materials; `cargo xtask compat verify`
fails closed on content-pin drift; `crates/clonk-core/src/version.rs` is the
single gating version.

### Presentation

Painter order, coordinates, sampling, blending, gamma placement, and resource
state equal the oracle. GPU sampling is verified by readback against the
software renderer with the one-byte cross-driver tolerance documented in
`docs/RENDERING_PARITY.md`. Every opt-in presentation divergence is forced off
and every first-run configuration default equals the C++ default.

Held: `docs/RENDERING_PARITY.md` and the F9 reference-capture pixel-parity
suites in `crates/clonk-frontend`.

Pending: clonk-org/clonk-rs#587, C++/Rust presentation capture diffs taken with
the profile active.

### Save and replay

Savegames carry the pinned `C4XVer` header and the `C4GameSave` component
layout; recordings use the `C4Record` binary format
(`RCT_CTRL`/`RCT_CTRL_PKT`/`RCT_FRAME`/`RCT_END` with 2-byte chunk heads).
Interchange with a stock C++ build is claimed only as far as the stated evidence
goes — format round-trips are not a claim that a C++ build will restore the file
into the same state.

Held: the `C4Record` format round-trips in `crates/clonk-engine/src/record.rs`
and the savegame component serialization tests in
`crates/clonk-app/src/main_tests/saves.rs`.

Pending: clonk-org/clonk-rs#524 (save serialization and restore ordering) and
clonk-org/clonk-rs#527 (resync and post-mortem recovery).

## Accepted divergences

These are permanent, understood differences from the oracle. Each row says what
the profile does with it. `reverted` rows are the profile's whole job;
`kept` rows are differences that cannot change what a C++ peer computes, so
reverting them would cost behaviour and buy nothing.

### Simulation

| Id | Action | Disposition |
| --- | --- | --- |
| `sim-game-tick-38fps` | reverted | The default tick is C++'s parameterless `SetGameSpeed` value, integer `1000 / 38 = 26` ms, rather than the 28 ms application timer `C4Game::OpenGame` installs. A 28 ms timer is capped at 35.714 updates per wall-clock second even on a free CPU, which cannot meet the product requirement that Hazard hold 38 updates/s. Offline play and recordings advance about 7.7% faster in wall time; the state after a fixed frame and control sequence is unchanged. **Product decision approved 2026-08-09**, recorded at `crates/clonk-engine/src/lib.rs`. The profile restores the 28 ms timer. |
| `sim-pxs-syncclearance` | kept | C++'s PXS `SyncClearance` copies a surviving chunk pointer downward without clearing the moved-from slot, aliasing one allocation into two slots — double processing and a double delete. Rust transfers unique ownership and clears the tail. This is an undefined-behaviour boundary in C++, so Rust's single-copy survivor order is authoritative and the profile does not reproduce the alias. Owned by `parity/README.md`. |
| `sim-team-account-shared-base` | reverted | With the TeamAccount rule (`TACC`) in play, `FindBase` falls back to a non-hostile player's base once the asking player owns none, so "back to base" and the wormhole reach an ally's base instead of failing. `C4Game::FindBase` matches `Base == iPlayer` exactly, with no team, alliance or rule test (`C4Game.cpp:3732-3745`), so this widens a lookup the oracle keeps strict. Own bases keep their existing priority and master-list order — the ally is only the lower-priority fallback the issue asks for — and the relation is `!Hostile`, the one the rule's own `ACNT` object maintains its alliances with and the one `C4Game::FindFriendlyBase` already applies to buying and selling. Base lookup drives command AI, so the profile clears it and restores the exact owner match. **Product decision approved 2026-08-24**, recorded at `crates/clonk-engine/src/compat/objects.rs`. |
| `sim-s2-terminal-params` | kept | S2 map-generator boundary inputs: C++ turns a negative Mandel alpha into a huge `uint32` iteration budget, divides `Gradient` by a zero width, and takes a remainder by zero at `alpha=-2`. Rust bounds the iteration count, substitutes a denominator of one, and returns false. These inputs are excluded from the differential runs. Owned by `parity/README.md`. |

The two `kept` rows are the only sync-relevant differences the profile carries,
and both sit where the C++ behaviour is undefined rather than merely different.
Everything else determinism-critical is either reverted or blocked.

### Control

| Id | Action | Disposition |
| --- | --- | --- |
| `ctrl-async-default` | reverted | `Network.ControlMode` defaults to `CNM_Async` (2) rather than C++'s `CNM_Decentral` (0), which `C4Config.cpp:540` calls the standard mode and `C4GameOptions.cpp:93` labels experimental. Only the default differs — the mechanism is a faithful port — but the value is **synchronized**, so it decides whether the host bounds its wait for a straggler and drops that client's input for the tick instead of pacing every participant on the slowest peer. The profile applies C++'s value as a non-persistent overlay at session construction; the saved key is never rewritten, and a mid-round edit cannot move a running session between modes. |
| `ctrl-keyup-release` | reverted | Outside the profile, classic (non-AutoStop) control sets emit a synchronized `Control*Released` on key-up so scripts can latch steering. `C4Game::LocalControlKeyUp` routes a key-up only for AutoStop players, so C++ never delivers a release in classic style. The profile suppresses the emission. |
| `ctrl-onmenustep-coms` | kept | Before a menu selection move the port offers the horizontal `COM_MenuLeft`/`COM_MenuRight` pair to the object's script through `OnMenuStep`. In C++'s single-column menu the horizontal pair carries nothing the vertical pair does not, and the callback is inert for content that does not implement it and answer true. |

### Transport

Transport divergences are host-local or browser-local and do not change
synchronized data. The profile keeps the quality improvements below and
restores C++'s definition-transfer default.

| Id | Action | Disposition |
| --- | --- | --- |
| `transport-definition-transfer-limit` | reverted | `Network.MaxLoadFileSize` defaults to 256 MiB rather than C++'s 100 MiB so large classic definition families can be transferred. The profile restores 100 MiB; an explicitly configured host limit wins in either mode. Owned by clonk-org/clonk-rs#945. |
| `transport-presend-envelope` | kept | The host sizes the control PreSend horizon from a decaying delivery-time envelope instead of C++'s 1/150 EWMA of the mean control-send time, so a jittery link no longer stalls every participant on about half its ticks. |
| `transport-udp-repair-interval` | kept | Reliable-UDP repair re-request damping is 250 ms rather than C++'s one-second `iReCheckInterval`. Only a repair request that is itself lost is affected. |
| `transport-lan-probe-interval` | kept | LAN discovery probes on its own shorter interval instead of C++'s single 30-second countdown that refreshes the LAN list and the masterserver row together. The masterserver keeps the oracle interval. |
| `transport-lan-failed-ref-backoff` | kept | A failed LAN reference query backs off for the error row's lifetime instead of C++ re-probing a refusing host every pass and stacking duplicate rows and connection attempts. |
| `transport-announce-burst` | kept | A host repeats its discovery announce on a burst interval after opening; C++ announces once and stays silent until probed, so one lost multicast datagram hides a C++ host until the browser's next probe. |
| `transport-http-client` | kept | Update and league HTTP runs on a configured `reqwest` client rather than a hand-written libcurl twin. The residual is that `reqwest` cannot emit an HTTP/1.0 request line, so a version-distinguishing server sees HTTP/1.1 with `Connection: close`. |

### Presentation

| Id | Action | Disposition |
| --- | --- | --- |
| `pres-remaster-family` | reverted | `Graphics.Remaster` is the master switch for ten opt-in presentation divergences (HighDpiCursor, Mipmaps, SmoothLandscape, FineFogOfWar, HDExactBlits, ShaderLandscape, LoaderAspect, SnapTextToPixels, SkyDither, SmoothPresentation). Each is off by default; the profile forces the whole family off. |
| `pres-render-inactive` | reverted | `Graphics.RenderInactive` defaults to both bits so an Alt-Tabbed game keeps drawing; C++ adapts `Console` alone. Only the default diverges — a written value is honoured verbatim, and `RenderInactive=2` restores C++ exactly. The profile reverts the default at startup, once the command line has resolved the requested profile, and still honours a written value under either profile: no overlay in the profile overrules a choice the player made explicitly. The advanced-config editor keeps materializing the port default, because the profile never rewrites a saved key. Owned by clonk-org/clonk-rs#57. |
| `pres-first-run-scale` | reverted | The first-run application scale is seeded from the monitor's pixel density; C++ starts every install at `Scale=100`, which on a 2x panel is an 800x600 device-pixel window with a 14px font. |
| `pres-scale-cap` | reverted | The Options scale spinbox and slider cap at 400 rather than C++'s 300, so a 4x panel can express the classic 800x600 logical layout. The slider mapping and scale-test flow are unchanged. |
| `pres-monitor-selection` | kept | `Config.Graphics.Monitor` selects the startup monitor. The oracle's SDL/GL build stores the row but never reads it back, so the default and every value below it keep the pre-existing behaviour; only an explicit positive index diverges. |
| `pres-main-version-text` | kept | The startup main dialog draws the port's release version in C++'s exact placement, font, colour, and markup, where C++ draws `C4VERSION`. Protocol identification and content gating still use the engine compatibility version. |
| `pres-new-player-name` | kept | The new-player dialog starts from the localized rank-0 name; C++ hardcodes the German "Neuling" for every language. The `Player.txt` omit-if-equal write default and the missing-`Name=` read fallback both stay "Neuling", so the file round-trips byte-identically with C++. |
| `pres-detail-governor` | kept | A presentation-detail governor drops fire-particle drawing and gamma steps from measured cost, where C++ exposes the same two steps only as static config. It skips the *draw* of a burning object's flames rather than stopping emission, so the engine's particle list is untouched and clients at different detail levels stay in lockstep. |
| `pres-draw-time-share` | kept | One application pass may drain a clamped 250 ms simulation backlog but reserves a share of the pass for drawing; C++ runs at most one `Game.Execute` per pass, so a machine slow enough to spend the whole pass simulating never repaints. Simulation state is unchanged by the split. |
| `pres-userdata-directory` | kept | User-data directories are named "Clonk Rust" while the window caption stays "LegacyClonk". A C++ install's user directory is never read or written by the port. |

### Save and replay

| Id | Action | Disposition |
| --- | --- | --- |
| `save-mission-access` | kept | `Config.General.MissionAccess` is persisted as soon as the shared mission list the engine mutates differs from the file. C++ leaves the write to `Config.Save()` on a clean quit, so a round that ends any other way relocks a mission C++ would have kept. Owned by clonk-org/clonk-rs#50. |

## Port content appends

Eight port-authored `#appendto` scripts live in `planet/System.c4g` and change
what shipped definitions do. They are content, not engine, but they are just as
visible to a C++ peer — seven of the eight are determinism-critical — so the
profile disables all eight and the shipped scripts run unmodified.

| Id | Target | What the append changes |
| --- | --- | --- |
| `content-append-bird-flight` | `BIRD` | Replaces the shipped four-coin-flips-per-tick bird steering with a continuous flight controller (separation plus weak alignment). |
| `content-append-airbike-steering` | `AB5B` | Hold-to-steer airbike handling with a double-integrator float model. Every test has an A/B twin pinning what LegacyClonk does with the append removed. |
| `content-append-gped-remote` | `SF5B` | Keeps the pilot parked while the GPED steers an airbike, answering the stale single-coms that Jump'n'Run control would otherwise turn into movement. |
| `content-append-guided-missile` | `RL5B` | Stops a guided rocket's turn when the steering key comes up; the shipped launcher latches the turn into the missile's command local until a straighten key clears it. |
| `content-append-sft-release` | `SF5B` | Completes the missing `Control*Released` pair for the Eke SFT's forwarded controls, so the airbike and the selected item learn that a steering key came up. |
| `content-append-fow-reveal` | `_FOW` | Lifts Dragon Rock's map-authored shadow volume before a Clonk enters it; the shipped generator overrides the Clonk's own light and holds interior objects inactive until it removes itself. |
| `content-append-mars-capsule` | `BASE` | Pays for a ClonkMars supply order atomically with an error report; the shipped commit spends one item at a time silently and abandons the rest when the first does not fit. |
| `content-append-gather-order` | `CLNK` | Adds a "Gather" context row to an owned crew member, listing one entry per loose item type it can both walk to and carry home and queueing Get/Enter for each in one batch. LegacyClonk has no order of this shape, so a spare crew member is either driven by hand or left as a backup. It sits here rather than under port-only features because it reaches the control stream: the row issues ordinary player commands through the existing queue, exactly as if each had been given by hand. Owned by clonk-org/clonk-rs#334. |
| `content-append-menu-range-row` | `MS4C` | Collapses a ClonkMars range choice from three rows into one row with primary/secondary stepping. Presentation only. |

## Port-only features

Features with no C++ counterpart at all. None of them can reach the simulation
or the control stream, so the profile keeps them rather than removing
functionality for a difference no peer can observe. Anything that *could* reach
lockstep would be a divergence in the table above, not a feature here.

| Id | What it is |
| --- | --- |
| `local-voice-chat` | Proximity voice chat: microphone input, the push-to-talk rebind dialog, the `Voice.*` config rows, and the "Audio" options group placed in the slack below C++'s own grid. Owned by clonk-org/clonk-rs#452. |
| `local-stats-overlay` | The `Graphics.ShowStats` diagnostics overlay and its default-unbound toggle key. C++ draws exactly one frame rate on the upper board and has no overlay. Owned by clonk-org/clonk-rs#158. |
| `local-dock-icon` | The per-platform dock/taskbar icon; `C4FullScreen` has no reconciliation step to match. |
| `local-f11-screenshot` | F11 as a screenshot shortcut; C++ maps F11 as an ordinary physical key. |
| `local-update-check` | The in-app update flow: an incoming `.c4u` package is refused rather than extracted and executed, an engine change gets its own message instead of reading as "no update", and the up-to-date case states the running version. |
| `local-app-icon` | Windows icon slot 0 is the port's own mark; C++ puts `lc.ico` there. The thirteen file-class icons are the engine's own, recovered from the pinned snapshot. |

## Unsupported combinations fail closed

The profile refuses rather than approximates. Each rule below names the
combination, what happens, and what the refusal rests on.

| Id | Combination | Behaviour |
| --- | --- | --- |
| `fc-profile-peer` | A peer that does not advertise the profile, or advertises a version this build does not support. | Refuse the connection with the named reason. Never fall back silently to the normal profile or to an unproven mixed session. |
| `fc-build-mismatch` | A peer whose `PID_Conn` build differs from the pinned build. | Refuse the connection — this is C++'s own admission rule, which compares only the build against `C4XVERBUILD` (`C4Network2.cpp:1291-1299`). |
| `fc-content-drift` | The content submodule pin or the in-repo `planet` tree differs from the manifest pins, or a scenario is loaded from any other content source. | Refuse to start in the profile. `cargo xtask compat verify` fails the gate on pin drift, so a content bump must restate and re-verify the contract in the same change. |
| `fc-readiness` | Any promise evidence is `pending`, or any divergence is `open-gap`/`blocked`. | The profile may not be advertised as compatible. It is either not offered, or offered as explicitly experimental — never as "legacy mode". |
| `fc-save-version` | A savegame, recording, or scenario whose `C4XVer` header or record type does not match the pinned engine version. | Refuse to load, exactly as C++ does. No attempt to repair or reinterpret foreign-version data. |
| `fc-config-conflict` | A saved configuration sets a profile-forced key (the Remaster family, `RenderInactive`, the scale cap, the first-run scale seed, or the game tick delay). | The profile value wins and shadows the saved value for the duration of the profile. The saved value is never merged in, and the normal profile is left untouched on exit. |

## Readiness: what the profile may not claim today

`fc-readiness` is the rule that keeps this document honest, and today it says
no. `cargo xtask compat verify` prints the count it acts on: the pending
evidence entries and the blocked divergences. While either is non-zero the
profile must not be presented to a player as compatible.

Seven open gaps currently block it, all determinism-critical:

| Id | Owner |
| --- | --- |
| `sim-script-var-negative-slot` — `Var(n)`/`Local(n)` with a negative index: C++ clamps to slot 0 (`C4ValueList::GetItem`), the port's VM addresses a distinct key. | clonk-org/clonk-rs#523 |
| `sim-script-nested-local-snapshot` — a call into an object whose own script is already in flight starts from the pre-call local snapshot; C++ keeps named locals on the `C4Object` and would read them live. | clonk-org/clonk-rs#523 |
| `sim-script-unwind-args` — when a script callback errors mid-call the port unwinds with the original argument values; C++ keeps the parameter mutations made before the error. | clonk-org/clonk-rs#385 |
| `sim-findobject-layer-bbox` — `FindObject`'s `Layer` condition is unmodeled on host objects, and shape tests use the vertices bounding box rather than the layered shape. | clonk-org/clonk-rs#384 |
| `sim-reloadparticle-io-failure` — `ReloadParticle` reports true when a reload passes all four C++ checks and then fails on I/O; C++ reports false. | clonk-org/clonk-rs#384 |
| `sim-containment-cycle-spawn` — a genuine containment cycle at spawn time: C++'s two-phase denumeration keeps the mutual containment, the sequential spawn model breaks one edge. | clonk-org/clonk-rs#518 |
| `sim-reload-graphics-dangling` — after a definition-graphics reload, an object that can neither re-resolve its graphic nor fall back to its definition is left holding a dangling name; C++ `AssignRemoval`s it. | clonk-org/clonk-rs#384 |

Plus seven pending evidence entries naming six issues —
clonk-org/clonk-rs#585 (simulation), clonk-org/clonk-rs#586 (control and
transport, once each), clonk-org/clonk-rs#583 (transport),
clonk-org/clonk-rs#587 (presentation), and clonk-org/clonk-rs#524 and
clonk-org/clonk-rs#527 (save and replay).

## Changing this contract

`cargo xtask compat verify` is a required landing gate. It rejects a manifest
that does not parse, a pin that has drifted from `clonk-core` or from the
`content` gitlink, a missing or unknown contract area, a divergence with no
owner or no citation, a determinism-critical divergence that never says what
C++ does, a fail-closed rule with no stated basis, an `open-gap` that is not
`blocked`, a bare `#N` issue reference, a citation naming a file that no longer
exists, and an entry that appears in the manifest but not in this document.

That last check is what keeps the two halves together. Adding a divergence,
a port-only feature, or a fail-closed rule means adding it here too — and the
practical consequence is the important one: **you cannot land a deliberate
difference from C++ without writing down who decided it and what it costs.**
