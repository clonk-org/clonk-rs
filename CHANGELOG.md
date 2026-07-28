# Changelog

All notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-07-28

### Bug fixes

- Keep plain arrow keys inside the focused scenario search edit
- Copy runtime asset groups recursively
- Widen the GPU backend set instead of aborting startup without an adapter
- Tag a release only after every platform builds
- Narrow the chunk window on the client that is actually downloading
- Stop extending the async deadline for a persistent straggler
- Publish resources in 10KiB chunks to unblock control behind bulk
- Size presend from measured control lateness, not ping alone

### Documentation

- Record enhanced scenario search as a deliberate C++ divergence and its accessibility and IME gaps
- Describe the game and show it in the README
- Attribute the per-commit engine measurements and the gate environment traps
- Record the texmap identity measurement and close its open gap
- Record the measured frontend allocation savings and the texmap name compare gap
- Record the low-power hardware profile, its measurements and the new levers
- Restore the landscape-extent hook's own doc comment
- Fold the unreleased 0.2.0 notes into 0.2.1
- Describe the real packaging outputs per platform

### Features

- Search the full scenario catalog live with normalized metadata ranking, conservative typo tolerance, folder context, result counts, and recoverable no-results
- Reduce presentation detail automatically when drawing cannot hold the frame budget
- Reserve wall-clock for drawing and force a repaint floor on slow hardware
- Hold a draw floor while catching up and pin the control-rate limit
- Carry capability announcements through the session transport
- Negotiate port protocol capabilities without breaking cpp interop
- Announce a lockstep stall instead of freezing silently
- Add a report-only chaos harness with recorded baselines
- Simulate whole lockstep sessions with per-client cpu profiles
- Model link capacity, queueing and competing traffic in the sim
- Construct plain procedure action specs through one constructor
- Compile and register a script definition in one engine call

### Performance

- Identify the texmap name tables instead of comparing them every frame
- Hash the per-frame and script name tables without a per-process seed
- Group particles by layer once per object pass
- Skip re-cloning the landscape cache grid it is already anchored to
- Defer the surface pixel plane until something touches pixels
- Compare C4 material names without owning their folded bytes
- Borrow the landscape for read-only host queries instead of copying it
- Narrow the per-peer chunk window while a game is running
- Spend control redundancy only on peers that report loss

### Refactoring

- Extract the per-particle draw from the layer walk
- Promote the link impairment model into a library module
- Build app runtime tests from one shared game app fixture
- Build compat test world contexts from one shared scaffold
- Build categorized test object scopes from one shared default
- Build render test graphics systems from one shared fixture
- Build compat test world objects from one shared default
- Fill unset struct fields from their default instead of restating them
- Build definition metadata from its derived default
- Give crew roster entries their cpp default construction
- Build compat test object scopes from one shared default
- Build command test runtime contexts through the shared builder
- Name the shared command runtime context builder for every command
- Run pending command continuations through one shared seam
- Share one scaled text caret across the classic dialogs
- Assemble the script host vm once for every call path
- Share one host object context builder across definition callbacks

### Styling

- Wrap the landscape cache anchor test density map
- Order the chaos imports

### Testing

- Refresh the shipped portrait census for the current content
- Assert script compilation through one shared helper

## [0.2.1] - 2026-07-28

Includes everything prepared for 0.2.0, which was never published.

### Bug fixes

- Package the game content notice from the content submodule
- Let the release override survive a failed CI lookup
- Stop packaging the licence files removed from the tree
- Keep breaking changes pre-1.0 in the release version bump
- Restore Hazard bullet collision sweeps
- Preserve Hazard bullet trajectories
- Keep spaces in scenario search
- Evaluate scripts in the active receiver context
- Abort startup when host times out
- Preserve full-size scenario button highlights
- Deduplicate construction material messages
- Gate PNG decoder import to tests
- Match the legacy portrait selector
- Report network scenario loading progress
- Expose saved games in scenario browser
- Lower unresolved failsafe calls like C++
- Make lobby text readable on black
- Decode network handshake rejection messages
- Match base overlay geometry to source definitions
- Hold a scenario batch Enter until its queued container exists
- Walk the master object list in FindObject2 and its sector lists
- Seed the new-player name from the localized rank ladder
- Draw the startup new-player form like C4StartupPlrPropertiesDlg
- Carry ActMap Sound into scenario-loaded action specs
- Send a third copy of each control datagram on lossy links
- Coalesce the one-second timer backlog like the C++ oracle
- Draw network catch-up passes that end on a skipped frame
- Stop one lossy or congested peer stalling every participant
- Size PreSend from the control delivery envelope instead of its mean
- Restore the workspace build after the rand 0.9 upgrade
- Keep the drawn audibility when an object moves before its sound starts
- Show C4Script log output on the in-game message board
- Enter a container the same call created after its content
- Accept WAVs whose RIFF length overruns the file [behavioral]
- Restore scenario/core.rs that .gitignore silently excluded
- Reinitialize the loader screen on every return to PreInit [behavioral]

### Continuous integration

- Add an auditable override for releasing past a red gate
- Release daily and give the parity gate room to finish
- Release on a weekly schedule without manual steps
- Re-enable the full parity gate now that content is public
- Automate tagging, building and publishing releases
- Add conventional-commit release preparation
- Add a submodule-free dependency guard
- Let Renovate open CVE fixes outside the monthly window
- Stop Renovate desyncing the pinned Rust toolchain [behavioral]
- Replace Dependabot with Renovate [behavioral]
- Stop running until the content submodule is reachable [behavioral]

### Dependencies

- Update rust crate rand to v0.9.3 [security]
- Update rust crate anyhow to v1.0.103 [security]
- Update rust crate rand to 0.9 [security]
- Update rust crate time to v0.3.47 [security]

### Documentation

- Remove private oracle checkout references
- Record the localized new-player name seed as a deliberate divergence
- Record the netplay latency divergences and their test gaps
- Make AGENTS.md the steering file and drop CONTRIBUTING.md

### Features

- Default the network control mode to async so one slow peer cannot stall everyone
- Draw the port version on the startup screen
- Play the looping ActMap action sound [behavioral]

### Performance

- Cut reliable-UDP re-ask damping to 250ms [behavioral]
- Optimize the shipped release profile and use mimalloc
- Size sectors from the landscape extent, not a shell copy

### Refactoring

- Apply workspace rustfmt
- Widen the deferred-enter helper to crate visibility
- Single-source the engine version constant
- Share the horizontal book scrollbar and startup draw helpers
- Move docs, test fixtures, and renovate config out of the repo root
- Carry the ActMap Sound field into ActionSpec [structural]
- Delete the unreachable save-browser rendering path

### Styling

- Restore rustfmt formatting [structural]

### Testing

- Refresh the shipped portrait census for the new content packs
- Match the license baseline to the rewritten copyright line
- Cover Eke missile scheduled explosions
- Accelerate Deep Sea construction coverage
- Pin that control forced past its tick is discarded, never replayed
- Measure what one slow peer costs the session under each control mode
- Schedule link_impairment burst loss in time rather than per datagram
- Model lockstep playout and control redundancy in link_impairment
- Freeze sector query ordering before touching the map path
- Add an impaired-link lockstep control harness
- Sweep flame population in the combat harness
- Add MeltMe simulation profiling harnesses

### Structural

- Group three field clusters out of Engine into sub-structs
- Split main.rs into five #[path]-mounted parts
- Move the Object and Definition clusters out of lib.rs
- Split scenario.rs production into child modules
- Split command.rs production into child modules
- Collapse the ok_or_else host-context preambles
- Collapse the match and map/unwrap host-context preambles
- Collapse the repeated host-context preamble in compat
- Split impl Engine into 19 #[path]-mounted area files
- Move lib.rs inline test modules to #[path] files
- Splice scenario.rs test body into byte-verbatim parts
- Splice command.rs test body into byte-verbatim parts
- Splice compat.rs test body into byte-verbatim parts

New sections are prepended by `scripts/prepare-release.sh`.

## [0.1.0] - 2026-07-24

Initial release: Windows, macOS (Apple Silicon and Intel) and Linux builds of
the Rust port, each shipping the engine, launcher, base content and the
authorized Eke Reloaded and ClonkMars packs.
