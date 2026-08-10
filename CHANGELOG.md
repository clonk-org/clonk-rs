# Changelog

All notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.6] - 2026-08-10

### Performance

- Split release candidate prebuild from packaging (#247)
- Halve release candidate build time (#245)

## [0.9.5] - 2026-08-10

### Bug fixes

- Release retired player profiles for rejoin (#239)
- Keep repeated sky tiles contiguous (#237)

### Testing

- Characterize inherited construction pathfinding deadlock (#242)

## [0.9.4] - 2026-08-10

### Bug fixes

- Preserve zero-sized spell world faces (#232)

### Performance

- Publish qualified releases within two minutes (#233)
- Keep 24-player Hazard games responsive (#229)
- Keep Hazard play responsive at four players (#228)

### Testing

- Cover Dragon Rock network hostility restoration (#230)

## [0.9.3] - 2026-08-09

### Bug fixes

- Restore network savegame players and construction state (#218)

### Performance

- Batch retained definition particles (#225)
- Keep the C++ control horizon when no ping can size it (#223)

## [0.9.2] - 2026-08-09

### Performance

- Send reliable UDP data once (#211)
- Reduce full-mesh network latency (#210)

## [0.9.1] - 2026-08-08

### Performance

- Halve benchmark execution time (#207)

## [0.9.0] - 2026-08-07

### Features

- Run a headless dedicated server with no window or render device (#199)
- Wire console Draw mode and viewport scrolling into the running app (#198)

## [0.8.2] - 2026-08-06

### Bug fixes

- Resolve a savegame Origin against the data root that spells it (#195)
- End the league registration when the application quits (#191)
- End the network session when a round is torn down (#190)
- Stop the config file overriding the netdlg Internet toggle and masterserver row (#189)
- Release the launcher window before the event loop returns (#185)
- Keep the network dialog searching after an abandoned join (#182)

### Documentation

- Correct which exits actually skip the league End (#192)

### Testing

- Stop packed-group round-trip checks depending on the wall clock (#188)

## [0.8.1] - 2026-08-05

### Bug fixes

- Default unreadable Teams.txt numbers instead of failing the load (#180)
- Stop a client's own control lookahead from forcing a fast-forward (#179)
- Say why a session ended instead of ending the log mid-stream (#178)
- Keep the game window drawing while it is unfocused (#177)
- License the workspace source under MIT instead of ISC (#175)
- Release the developer windows before the event loop returns (#173)
- Share one wgpu instance across every window (#171)

### Features

- Add an opt-in diagnostics overlay for render rate and control latency (#176)

### Testing

- Pin that the shared instance lookup goes through the registry (#172)

## [0.8.0] - 2026-08-05

### Bug fixes

- Emit engine fire particles from burning objects (#162)

### Features

- Trail SmokeRate smoke from burning objects and make the fire detail rung bite (#168)

## [0.7.1] - 2026-08-05

### Bug fixes

- Compose the landscape with every solid mask lifted (#166)
- Yield a simulation burst when the next graphics opportunity is due (#163)
- Keep solid-mask bytes out of the render-dirty lineage (#161)

### Testing

- Fail loudly when content changes a function the menu overrides replace (#160)

## [0.7.0] - 2026-08-04

### Bug fixes

- Keep the startup game search running when a join password prompt is abandoned (#154)
- Report an operating-system key repeat on every target, including macOS (#152)
- Render a failed client join without a duplicated connect caption (#149)
- Draw the product logo on ClonkMars upper boards (#148)
- Demote the unresolvable-sound-name log to debug like C4SoundSystem (#147)
- Resolve a global func body against the engine, not its declaring host (#143)
- Resolve inherited against the owner list C4Aul searches (#140)
- Keep network peers on one material order by loading packed resource images (#139)

### Documentation

- Point the test globs at the renamed tests (#151)
- Replace private tracker audit ids with the facts they stand for (#146)
- Qualify every issue reference with its owner and repository (#144)

### Features

- Show the running order total and the player's wealth while ordering (#156)
- Step a Menu2 range with the arrow keys and price its order before delivery (#155)
- Collapse a ClonkMars order row to one row per product (#153)
- Report an unresolvable inherited call at link time like C4Aul (#145)

### Refactoring

- Name tests for the behaviour they pin, not the item that prompted them (#150)
- Remove the unreachable startup-menu frame cache (#141)

## [0.6.5] - 2026-08-04

### Bug fixes

- Keep LAN discovery and the host reference server alive when the multicast join fails (#137)
- Run Fx callbacks on the effect command target (#136)
- Treat an empty Sound name as a lookup that finds nothing (#135)
- Draw the upper board scenario title with markup (#132)
- Wrap Abs at i32::MIN like the C++ two's-complement negation (#127)
- Report a tolerated script error above its own call frames (#130)
- Dispatch the configured NetStatsToggle key (#128)
- Wrap C4Script integer subtraction on overflow like C++ (#126)
- Reproduce the wrapped C++ Sqrt correction steps (#124)

### Continuous integration

- Stop publishing a dependency-guard cache no ref can restore (#125)

### Documentation

- Claim a GitHub issue before working it (#133)
- Require opening and shepherding a pull request for every change (#129)

## [0.6.4] - 2026-08-03

### Bug fixes

- Generate admissible dependency pull request titles (#93)
- Align Windows DX12 dependency types (#92)
- Remove pixel-less column collision fallback (#85)
- Preserve mapped UDP destinations on IPv6 sockets (#91)
- Refresh liquid state after SetPosition (#84)
- Align AddMessage with C++ append semantics (#83)
- Update the pixels rendering stack (#86)
- Correct Mars oxygen alarm behavior (#82)
- Route effect conversion warnings through debug log (#81)

### Testing

- Stabilize post-join address assertions (#97)

## [0.6.3] - 2026-08-02

### Bug fixes

- Install updates from within the app (#79)
- Reoffer eliminated player files in join menu (#78)
- Tile oversized landscapes for retained rendering (#77)

## [0.6.2] - 2026-08-01

### Bug fixes

- Preserve pre-strict3 effect callback values (#70)
- Preserve pre-strict3 effect check parameter values (#69)
- Reanchor frame timer after long stalls (#67)
- Keep player menu available after elimination (#68)
- Retain runtime join data until host timer (#66)
- Fall back to CPU for oversized GPU textures (#65)
- Persist earned mission access as soon as a round grants it (#52)

### Continuous integration

- Add a five-minute landing pipeline (#64)

### Testing

- Stabilize runtime dynamic timer assertion (#72)

## [0.6.1] - 2026-07-31

### Bug fixes

- Bind the IPv4 wildcard as the IPv6 wildcard so hosts reach IPv6 netpunchers (#49)
- Mute the unmappable Vulkan enum warnings (#46)

### Testing

- Pin oracle-faithful solid-mask shielding through a blast (#48)

## [0.6.0] - 2026-07-31

### Bug fixes

- Draw the first frame whatever RenderInactive says (#39)
- Let UpdateFlipDir own the FlipDir mirror instead of the renderer (#38)
- Discover SoundFonts in platform bank directories
- Attach the macOS Dock tile once the application has launched

### Continuous integration

- Reset an existing release branch instead of failing to recreate it (#42)
- Check pull request titles now that the merge queue squashes (#36)

### Documentation

- Document the MIDI SoundFont requirement
- Correct why joined option selections cannot arrive
- Fix stale steering-file claims and trim the parity section

### Features

- Make the scenario editor usable with viewport windows, editing and live reload (#35)
- Super-resolve the scenario icon strip (#37)
- Fly birds on a steered heading instead of per-second axis snaps (#34)
- Super-resolve the stretched startup menu art
- Group the update component archives under an update- name prefix
- Rotate through ten HarpoonRace relaunch messages

### Refactoring

- Share the component archive name prefixes between construction and parsing

### Testing

- Pin the joined lobby Options tab click

## [0.5.0] - 2026-07-30

### Bug fixes

- Size the options key labels with the C++ font zoom
- Draw the options key labels separator and glyph modulation
- Lay out the options control sheets with the C++ aligner walk
- Point the updater at the repository's new home
- Title the update check and name the version it found
- Preserve pinned history for script tests
- Install Pillow for CI script tests
- Stop an inactive shell from banking unbounded graphics deadline debt
- Resolve consolidated integration test targets
- Persist script-earned mission access at the config save surfaces
- Gate the network scenario selector on the live mission-access list
- Clamp script SetPosition by BorderBound, not the landscape surface
- Install the bundle icon when a macOS update is applied in place
- Give the launcher window and the Windows taskbar the product icon
- Give an unbundled macOS run the product Dock icon
- Cut the product icon from the logo's leading stone glyph
- Premultiply alpha across the product-icon downscale
- Flash the observer-menu hint when the ownerless viewport takes over
- Bind the local presentation to a synchronized join's own player
- Restore the lobby resource transfer window to the C++ byte budget
- Bound served resource chunks by the resource chunk size
- Report the packed group maker in network dynamic metadata
- Stop planning a bare package run for crates that own no test binary
- Acknowledge flushed part before route retirement
- Clear league status before shutdown completes
- Mark the shipped Eke global appends nowarn
- Break both presentation detail streaks on an in-budget pass
- Restore CI and automated releases
- Write the update package contents checksums
- Write a correct update-entry manifest instead of uninitialised bytes
- Keep SDL keydowns fresh on the macOS backend
- Defer participants and read the pending value first
- Defer the startup warning preferences to a save surface
- Defer the sound option writes to a save surface
- Flush deferred config when leaving the options dialog
- Defer mission access writes to a clean shutdown
- Widen overflowing ingame menus for the scroll bar
- Re-expand the user path on every native lookup
- Apply native recording and screenshot folder semantics
- Route debug log output by runtime debug mode
- Honor the RenderInactive bitmask when unfocused
- Size the async worker runtime from the configured thread count
- Title the native window with the engine caption
- Read runtime config scalars with the native numeric grammar
- Read runtime config booleans with the native grammar
- Resolve sound discovery from the selected config paths
- Stage network resources under the configured work path
- Wire the configured max resource search recursion
- Apply the configured ScrollSmooth to the runtime camera
- Stop clearing player controls on window focus loss
- Route F11 through the classic key registry
- Size Context ObjectRank rows by the live item height
- Show live remapped key names in the F1 help
- Localize omitted SetNextMission labels
- Fail closed on every mandatory HUD graphics facet
- Keep network test hooks out of release builds
- Fall back to the classic loader wildcard at startup
- Preserve literal IRC transcript text and query routing
- Match C4ChatControl input parsing and send errors
- Localize the IRC status transcript
- Make the IRC transcript scrollbar pointer-operable [M09-P3-L193-irc-native-scroll-windows.md]
- Bound IRC transcripts and scroll the channel nick list [M09-P3-L193-irc-native-scroll-windows.md]
- Show live protocol rates in the network status overlay [M09-P3-L190-network-overlay-protocol-rates.md]
- Draw evaluation goal pictures from the live goal object [M09-P3-L185-gameover-live-goal-picture.md]
- Give league evaluation rows their own rank icon column [M09-P3-L183-gameover-evaluation-row-parity.md]
- Lead each two-team evaluation list with its native team header [M09-P3-L183-gameover-evaluation-row-parity.md]
- Overlay the joined savegame crew on evaluation rows [M09-P3-L183-gameover-evaluation-row-parity.md]
- Draw the native scrollbar for overflowing evaluation text [M09-P3-L183-gameover-evaluation-row-parity.md]
- Compose the native league and settlement evaluation score labels [M09-P3-L183-gameover-evaluation-row-parity.md]
- Freeze player BigIcons at evaluation time [M09-P3-L182-gameover-early-bigicon-snapshot.md]
- Give SetMenuSize rows the native C4Menu Lines lifetime [M09-P3-L177-menu-setmenusize-rows.md]
- Localize every in-game menu caption and tooltip at runtime [M09-P3-L176-mainmenu-runtime-localization.md]
- Give the team menus the native normal grid and Tick35 refill [M09-P3-L175-mainmenu-team-style-refill.md]
- Show unknown client ids and the host acknowledgement marker [M09-P3-L174-client-info-unknown-ack.md]
- Expose the read-only joined lobby Options sheet [M09-P3-L171-joined-lobby-options-sheet.md]
- Accept the full classic gamepad raw event space [M09-P3-L163-gamepad-full-raw-event-space.md]
- Package a single macOS architecture when the second is not installed
- Verify the published content archive before publishing a release
- Refuse an uppercase content pin
- Carry a component's source release into the update plan
- Keep the retired Windows gnu triple on the update path
- Compile the packaging tool for Windows
- Show the event target on the diagnostic sinks
- Log fail-safe script recovery at debug level
- Keep the usable parts of a log filter directive
- Keep dependency log suppression when a filter is set
- Write default subscriber output to stderr
- Claim the logging init slot before opening the session log
- Keep the previous session log
- Mark debug and trace lines in the gui sinks
- Project message board prefixes from the event level

### Continuous integration

- Prepare releases through the GitHub App
- Gate the long jobs on the merge queue instead of every pull request
- Add Rust code coverage gate
- Resolve content from the content repository's own release
- Package macOS once as a universal build
- Gate releases on Windows and prove the crt against release binaries
- Gate the Windows configuration a release ships
- Build and package Windows on a native runner

### Documentation

- Record the options control sheet parity result and gamepad gap
- Require pull requests and describe the merge queue
- Log the unported SetPosition liquid update and the invented surface snap
- Record the icon defects closed across the three launch paths
- Log the unported join capacity gate and client deactivation
- Record identity-addressed viewport projections
- Record the console script-edit relink path
- Correct the shipped triple and engine build counts
- Record the completed evaluation-row parity [M09-P3-L183-gameover-evaluation-row-parity.md]
- Correct the shipped triple and engine build counts

### Features

- Compose the landscape in the fragment shader when opted in
- Build a shader landscape plan from resolved texmap slots
- Add the shader landscape and detail config seam to the retained renderer
- Target one options sheet from the headless menu dump
- Return network clients to the lobby when the host restarts
- Point content at the high-resolution crew sheets
- Install graphics variants and re-flow a definition's auxiliary sheets
- Install rendered high-resolution sprite packs into shipped definitions
- Embed the Windows executable icon resources in engine.rc order
- Present the startup screens at the display refresh period
- Port the definition reload load-what flags
- Port the definition file-monitor registration rule
- Port the definition reload sequence
- Implement the c4group update apply command
- Implement the c4group update generation command
- Port the update package entry diff
- Verify the update package group file checksum
- Read and write the c4u update core
- Project edited component hosts onto the scenario save
- Port the tools dialog open, clear and default state
- Address viewport projections by physical identity
- Port the console viewport window spec
- Publish the edit cursor mode change to the toolbox and cursor
- Port the edit cursor overlay draw list
- Port the viewport draw order and console overlay hook
- Port the freedesktop notification action encoding
- Install the Windows taskbar sink once the window exists
- Add the Windows taskbar progress COM sink
- Route changed files and particle reloads like the console
- Fan the console script over the selection and coalesce refreshes
- Port the tools page control order and enable rules
- Port the developer toolbox notebook and its hide lifecycle
- Port the definition reload refusal and object sweeps
- Resolve the ready check exactly once across dialog and toast
- Commit an edited scenario script and relink like the console
- Select the HTTP backend from Network.UseCurl
- Register the console shell as a developer window record
- Route landscape-mode changes through the draw-tool control
- Port the edit cursor's targeting, drag and drop-target gestures
- Expose the ordered object-inspection read model
- Compose the developer property panel text
- Gate the file monitor and validate reload payloads
- Model viewport player lock scroll ranges and input routing
- Model the console component host commit and save rules
- Add edit cursor mode cycling and context enablement
- Project viewport pointer coordinates per window identity
- Match changed paths to definitions like getbypath
- Defer runtime config toggles to a clean shutdown
- Add the keyed developer window registry
- Add the developer draw tool state machine
- Blit the device selector facets on the options control sheets
- Blit the classic key facets on the options control sheets
- Localize options control labels and port the key facet geometry
- Draw pressed scrollbar arrows and raise their gui sounds
- Repeat held scrollbar arrows and jump the thumb on track clicks
- Draw the overflow scrollbar in both menu paths
- Route engine object menu overflow through the shared scrollbar
- Unregister the windows file classes from c4group
- Complete the c4group command set except update packages
- Implement the c4group mutating commands and install it
- Add a c4group command line with the native parser
- Own the developer ordered edit selection
- Expose the developer landscape tool read model
- Register the windows file classes and clonk protocol
- Open a versioned about modal from the console help menu
- Report pre-window startup failures natively
- Persist the developer console window position
- Attach the product icon to both window shells
- Mirror loader progress to a taskbar adapter
- Show runtime log lines on the loading screen
- Honor graphics verbose object loading levels
- Honor the classic windows allocconsole policy
- Refuse effective-root startup on unix
- Install the Windows unhandled-exception crash handler
- Launch the classic editor from startup F6
- Accept the debug host and client classic shortcuts
- Apply the classic startup screen argument
- Apply the shared config Logging section to tracing filters
- Write Unix fatal-signal diagnostics before reraising
- Recover the original bundle path when macOS translocates it
- Warn on wild savegame player takeover
- Accept the extended SDL scancode name space in KeyConfig
- Honor the C4Object ViewEnergy bar timer
- Capture startup screenshots with bare F9
- Load About chrome captions from runtime resources
- Load startup options text from runtime resources
- Write FolderMap image dumps and resolve title fonts by height
- Activate the advertised NetDlg Alt mnemonics
- Port the netgetscen running chat command [M09-P3-L187-chat-netgetscen-command.md]
- Reference the content archive the content repository publishes
- Let a manifest entry name the release that publishes it
- Fuse the macOS release into one universal build
- Log a startup banner
- Log panics to the session log

### Performance

- Cut cold workspace builds below one minute
- Avoid unused full-history checkouts
- Preserve the default parity test graph
- Avoid duplicate dependency guard codegen
- Reduce CI rebuild and queue latency

### Refactoring

- Extract the texmap slot resolution from the landscape composer
- Drop a redundant cast in the font image blit
- Share the DrawLineDw port across the frontend crate
- Share the options ComponentAligner port across the crate
- Route parity through lightweight dispatcher
- Name the live bounds-check helpers for any caller, not just Exit
- Extract the shared product-icon composition into its own crate
- Rename the runtime flash builder to its resource-generic name
- Expose the maker MutableGroup will pack
- Wrap an over-long assertion in the refresh ceiling test
- Satisfy platform-specific clippy
- Move the scrollbar drawing into the shared module
- Share one classic scrollbar model across menus
- Format touched app test fragments
- Name the triple-alias pass for what it serves
- Make the content archive unbuildable by construction
- Share one lowercase-hex predicate
- Name the shipped executables in one place
- Test the engine debug gates before reading the environment
- Install one layered subscriber for every sink
- Define the script log target once
- Commit captured console bytes on flush

### Testing

- Synchronize empty sync release assertion
- Refresh the character-menu hashes for the high-resolution crew pictures
- Locate the owner-overlay reference crop instead of assuming the sheet origin
- Pin high-resolution crew art to one authored texel per device pixel
- Pin the high-resolution definition path against a real crew pack
- Synchronize dual-route reconnect lifecycle
- Detach allocconsole child before allocation
- Report a per-segment frame-time trend from the scenario profiler
- Expect the implemented update generation command
- Pin the native MaxRefreshDelay default across resolvers
- Pin Sec1 timer coalescing across long stalls
- Pin joined-lobby tooltip ownership across frames
- Regenerate the shared manifest fixture from the shipped triples
- Pin the level of the fail-safe script path
- Feed the message board the projected gui sink line

## [0.4.0] - 2026-07-28

### Bug fixes

- Replay lobby chat history for late joiners
- Resolve every component archive per target triple
- Check only pushed commits in the pre-push rustfmt hook
- Scope the scheduler test clock imports to unix
- Pin archive timestamps instead of inheriting the wall clock
- Clear the Windows-only lints in the launcher crates
- Run handle-less scheduler procs once on Windows
- Area-average save thumbnails instead of two-tap sampling
- Lay the launcher out in logical units and draw it with a vector face
- Read MODE_Action overlay facets through the source definition scale
- Honour --no-archive when packaging for macOS
- Decode MP3 through symphonia instead of the unsound minimp3 stack
- Update bytes and rand past their security advisories

### Continuous integration

- Publish the update components and their manifest with the release
- Check pushed files on rustfmt stdin so absent modules do not read as drift
- Run workspace tests through nextest
- Check formatting in a separate job
- Report every Windows suite instead of stopping at the first
- Test the launcher on a Windows runner

### Dependencies

- Lock file maintenance

### Documentation

- Record the host-order material slot gap blocking Linux replay goldens
- Simplify the issue templates down to a single bug form
- Record the high-DPI presentation divergences
- Record that point raster width does not track world zoom

### Features

- Check for updates from the startup menu
- Apply an update by swapping components with rollback and resume
- Download update components with cancellable progress
- Generate the update manifest from the emitted component archives
- Fetch update manifests over guarded https
- Journal an update apply so an interruption is recoverable
- Resolve update manifest and component archive urls
- Reject component archives whose entry names collide case-insensitively
- Add the client-side update core crate
- Search the full scenario catalog live
- Emit update components from the package command
- Emit content-addressed component archives and a signed manifest
- Map the staged layout onto update components
- Add a fragment-shader landscape material composer
- Blit HD definition art at one texel per device pixel
- Serve any font recipe natively and honour higher-resolution GUI sheets
- Add opt-in fog-of-war grid subdivision
- Raise the application scale ceiling to 400 percent
- Add opt-in aspect-filled loader backgrounds
- Add opt-in alpha-weighted landscape magnification
- Add opt-in mipmapped minification behind a remaster switch
- Add opt-in sub-LSB dithering for the sky gradient
- Seed the first-run application scale from the display density
- Add an opt-in high-DPI cursor tier ladder
- Turn guided missiles only while a turn key is held
- Synchronize key releases in classic control

### Performance

- Avoid redundant effect state clones

### Refactoring

- Apply rustfmt to files that drifted from the gate
- Drop manifest signing in favour of TLS and content hashes
- Extract a reusable deterministic zip writer
- Carry solid-primitive fragment options in one style value
- Build the event loop before resolving the initial window size
- Track incoming and requested updates separately
- Resolve the binaries directory from the install root

### Testing

- Pin that the update transport trait can be faked as an object
- Read back bundle-root groups only on macOS
- Make launcher and path assertions portable to Windows

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
