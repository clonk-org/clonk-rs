# Changelog

All notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.20.2] - 2026-08-26

### Bug fixes

- Compare the object fields the bridge carries but never diffed (#1161)
- Run the initial DoCon solid-mask update before the keep-bottom move (#1165)
- Stop diffing a construction value the bridge never collects (#1164)
- Stop a non-rotateable ChangeDef from re-mobilising the object (#1163)
- Stop the walk-rotation rdir write from arming Mobile (#1162)
- Report a construction divergence instead of its downstream probe (#1160)
- Report an in-liquid divergence instead of its downstream action (#1159)

### Documentation

- Point menu parity at new issues now its trackers are closed (#1156)

### Testing

- Fail a content bump that strands the dev-replay goldens (#1158)

## [0.20.1] - 2026-08-26

### Bug fixes

- Drop the unreachable nested-local snapshot divergence (#1147)
- Read the ift bit in the script wind lookup (#1143)
- Keep the facing when a swim exit falls into walk (#1138)

### Documentation

- Record the shadow-diff measurement traps (#1133)

### Features

- Bundle the Ultimate Clonk Compilation collection (#1144)
- Separate shared bases from team accounts (#1135)

### Performance

- Reduce simulation, script, and rendering overhead (#1140)

### Testing

- Pin the instable readback of a temperature-converted pixel (#1145)

## [0.20.0] - 2026-08-25

### Bug fixes

- Refuse a System group whose contents differ from the host's (#1073)
- Trace the map-creation draws the oracle already records (#1070)
- Resolve the installed material group past the scenario folder's own overlay (#1067)
- Compare the synced RNG ledger on every frame and report the slip (#1065)
- Trace the Rnd3 fill draws in the RNG differential probe (#1064)
- Stop reporting untransported effect state as a global divergence (#1060)
- Keep the reliable-UDP socket serving through a full send queue (#1052)
- Name the issue an unproven evidence entry already carries (#1043)
- Keep savegame clients activated through the load into GO (#1037)
- Build the bundled freetype crates natively on Windows (#1038)
- Bound record stream inflation and stop truncating a full buffer (#1023)
- Complete synchronized runtime joins (#1033)
- Bound serialized C4Value nesting instead of overflowing the stack (#1026)
- Match PXS malformed-load and invalid material state (#1014)
- Bound the packed group entry-table reservation by its image (#1022)
- Accept truncated JPEG entropy like the libjpeg oracle (#1015)
- Preserve object list chronology across creation callback phases (#1012)
- Retain startup player count in runtime joins (#1011)
- Retain mouse targets until move refill (#1010)

### Features

- Let the shadow diff state the host's fair-crew parameters (#1072)
- Record the trusted local system override as a compatibility gap (#1063)
- Declare the presentation capture screens masks and tolerances (#1062)
- Let LC_PIN_SEED freeze a network host's parameter seed (#1058)
- Build the pinned oracle against this tree for the shadow diff (#1057)
- Block the compatibility profile on the System.c4g identity gap (#1056)
- Restore the engine C ABI and build command the shadow-diff bridge needs (#1034)
- Run a compatibility session at the C++ in-game tick (#1039)
- Revert the inactive-draw default under the compatibility profile (#1045)
- Withhold the content appendto divergences under the profile (#1042)
- Tell a joining client its requested profile is unavailable (#1041)
- Refuse a join whose advertised compatibility profile cannot be matched (#1032)
- Share bases between allies under the team account rule (#1035)
- Advertise the compatibility profile a host actually claims (#1031)
- Port the C4Config version migration run after config load (#1029)
- Report compatibility blockers before a host claims the profile (#1028)
- Withhold the classic key-up release under the compatibility profile (#1027)

### Refactoring

- Restore the byte-verbatim test fragments rustfmt reflowed (#1076)
- State the fair-crew parameter the physicals tests depend on (#1075)

### Testing

- Differential-check config load mutation and save semantics (#1040)
- Pin sub-pixel crew positions through the dynamic state capture (#1036)
- Pin the save component set and restore-info ordering (#1030)
- Fuzz the legacy network packet and JoinData decoders (#1021)
- Fuzz scenario save and compiled-value parsers (#1025)
- Fuzz update manifests packages and apply paths (#1024)
- Pin the shipped invisibility timerless interval expiry (#1020)
- Pin the timer-less expiry path magic spells depend on (#1019)
- Pin that invisibility holds its time while its target is inactive (#1018)
- Pin the key that unlocks the Dragon Rock princess cage (#1017)
- Cover the shipped Dragon Rock cage unlock control path (#1016)

## [0.19.4] - 2026-08-24

### Bug fixes

- Preserve live effect callback lifecycle ordering (#1008)
- Match native C4Value map key equality (#1006)
- Preserve container lifecycle order and link identity (#1005)
- Retain players in lobby when host restarts (#1004)
- Synchronize fire state with cached ocf (#1000)
- Align pointer input with scaled presentation crop (#999)
- Reuse completed player resources after shadow expiry (#1002)
- Show savegame overwrite results in game (#1001)

## [0.9.4] - 2026-08-24

### Bug fixes

- Reuse completed player resources after shadow expiry (#1002)
- Show savegame overwrite results in game (#1001)

## [0.19.3] - 2026-08-23

### Bug fixes

- Refresh saved games before reopening scenario browser (#995)
- Skip zero-weight vertices in collision redirection (#994)
- Preserve material reaction lookup parity (#991)

### Performance

- Complete initial lobby joins within 500ms (#996)

### Testing

- Differential-check PXS lifecycle and execution order (#993)

## [0.19.2] - 2026-08-22

### Bug fixes

- Decide rejected speech before script continuation (#989)
- Complete native portrait selector parity (#988)
- Bound C4Script expression nesting instead of overflowing the stack (#976)
- Stop slicing past a value-less INI line (#975)

### Performance

- Halve Skies of Fire activation time (#986)

### Testing

- Fuzz the legacy WAV MIDI and RMID decoders (#974)
- Fuzz the legacy resource-text parsers (#973)

## [0.19.1] - 2026-08-21

### Bug fixes

- Expand the mouse pick box upward by addtop for short objects (#958)
- Carry the emitting object's position on attached sound calls (#956)
- Transfer large classic definition families (#953)
- Advertise the live runtime-join admission on a running host (#952)
- Bind portrait selector access keys to the caption's own letter (#951)
- Stop Escape from quitting the game at the main menu (#950)
- Use configured network identity for lobby clients (#944)
- Resolve definition packs from the directory holding the scenario pack (#942)
- Measure the connection acceptance window with a monotonic clock (#937)

### Testing

- Pin that a denied effect keeps its number until the next cycle (#954)
- Pin the dead font-atlas branch and fractional italic compounding (#949)

## [0.19.0] - 2026-08-21

### Bug fixes

- Pad font atlas cells and gutters with transparent white (#939)
- Complete runtime joins through chase catch-up (#938)
- Name the peer terminal reason in reliable-UDP write errors (#936)
- Load savegame origin materials for network hosts (#925)

### Features

- Offer an idle clonk a gather order for reachable loose items (#920)

### Testing

- Differential-check the save core adjustments against the c++ golden (#935)
- Pin that a horizontal contact discards the subpixel remainder (#930)
- Differential-check the builtin material reaction map against C++ (#934)
- Differential-check the dig-free circle walk against C++ (#933)
- Differential-check landscape material extraction against C++ (#932)
- Differential-check the c4value save type tags against the c++ golden (#931)
- Differential-check landscape material insertion against C++ (#929)
- Pin that a zero collection limit means unlimited (#928)
- Differential-check the corrode movement arm against C++ (#927)
- Differential-check the save runtime component sweep against the c++ golden (#926)
- Pin that the damage chain replaces its value and stops at zero (#924)
- Differential-check the poof movement arm against C++ (#923)
- Differential-check the incinerate reaction arms against C++ (#922)
- Pin the frontmost mouse candidate and the dead foreground pass (#921)
- Pin that the console halt buttons follow the live halt count (#919)

## [0.18.0] - 2026-08-21

### Bug fixes

- Compare c4group entry names by byte like native stricmp (#908)
- Defer the fair crew toggle like every other game option (#906)
- Announce the compatibility profile only where it governs the session (#905)
- Drop the update download bar when the transfer length is unknown (#902)
- Compare a null-object C4Value by tag like C++ (#858)
- Arm the client start barrier from JoinData so runtime joins run (#900)
- Match C4Group entry wildcards with backtracking like C++ (#848)
- Let a player control outrank a rebound pause chord (#897)
- Recover missing legacy definitions during startup (#887)
- End a cancelled download in the classic error modal (#893)
- Resolve a scenario's definition packs from its own folder (#892)
- Widen and clamp the portrait location dropdown like C++ (#871)
- Honor PointFiltering when upscaling portrait thumbnails (#869)
- Suppress the download bar when the transfer length is unknown (#857)
- Walk live effects when building the object info menu (#852)
- Open scenarios inside packed folder groups (#849)
- Resolve chop_action through the ActMap slot order (#836)

### Continuous integration

- Enforce merge queue latency budget (#839)
- Halve merge queue latency (#829)

### Documentation

- Record the live effect walk in the Info menu inventory (#862)
- Define the LegacyClonk compatibility profile contract (#851)

### Features

- Drag the portrait selector by its title and share one decode cadence (#883)
- Localize every portrait selector caption and size its location label (#876)
- Bounce the portrait selector title like a classic wooden label (#879)
- Raise the native context-menu sounds for the location dropdown (#873)
- Compute compatibility readiness from the contract (#891)
- Carry the compatibility profile in the game reference (#888)
- Apply the C++ control mode default under the compatibility profile (#872)

### Performance

- Avoid inspecting hidden scenario descendants (#890)
- Stop copying the container on every indexed assignment (#877)
- Stop snapshotting the effect list on every CheckEffect (#843)
- Sort host world storage order on read instead of every materialization (#807)

### Refactoring

- Separate the participant list rebuild from its disk write (#841)
- Remove residual dead and duplicate code (#805)

### Testing

- Differential-check the pxs savegame load path against the c++ golden (#917)
- Differential-check pxs casting against the c++ golden (#916)
- Differential-check the pxs insert arm against the c++ golden (#914)
- Differential-check the pxs conversion arm against the c++ golden (#913)
- Differential-check the pxs insertion arm against the c++ golden (#912)
- Pin the density gate on the builtin material reaction ladder (#911)
- Differential-check the per-tick pxs step against the c++ golden (#910)
- Pin the shipped english table as utf-8 so launcher ellipses survive (#909)
- Pin that alt mnemonics follow the translated access key (#907)
- Pin that mouse picking ignores the fore and background categories (#904)
- Pin that a late JoinData packet is dropped without a disconnect (#903)
- Differential-check the config language sequence against C++ (#855)
- Pin per-client independence of the resync throttle (#899)
- Differential-check the save policy matrix against C++ (#846)
- Pin update idempotence and unknown-source refusal (#898)
- Pin native startup text rasterization at every supported scale (#886)
- Differential-check the mouse cursor priority cascade against C++ (#838)
- Pin the effect removal reason for death against removal (#861)
- Pin the update request combined with a direct launch (#867)
- Pin that live tracing output reaches the loading screen (#896)
- Pin Info-menu markup geometry and cumulative italic at scale one (#865)
- Pin the serialized C4Value tag and array size rules (#894)
- Pin that scoreboard icon facets never sample neighbouring cells (#889)
- Differential-check the definition change sequence against C++ (#833)
- Measure where global AddEffect cost actually goes (#860)
- Pin the C++ wildcard exhaustion and backtracking rules (#882)
- Drive the shipped MWTH and FREZ thermal pair end to end (#864)
- Pin that an explicit launch failure exits instead of reconstructing startup (#881)
- Pin that overlapping Queron relaunches keep separate countdowns (#868)
- Refuse a dynamic whose metadata disagrees with its bytes (#875)
- Differential-check the object death sequence against C++ (#828)
- Let parity verify run comparators outside clonk-engine (#859)
- Differential-check the MWTH and FREZ thermal pair (#866)
- Cover every shipped Goal and Rule Activate family (#863)
- Differential-check the object removal teardown against C++ (#825)
- Measure how indexed array assignment scales with length (#842)
- Measure what deep sea volcanoes cost per frame (#840)
- Cover the shipped catapult launch and wagon cargo paths (#808)
- Pin that italic shears a mixed text and image row together (#854)
- Pin that C4Script arrays copy on assignment and argument (#853)
- Pin that both control styles steer the shipped submarine alike (#850)
- Differential-check the shape attachment search against C++ (#802)
- Drive the shipped SMIC ice crow summon end to end (#791)
- Pin the script player name draw bound and timing (#834)
- Pin the unspoken message format abort (#845)
- Differential-check the team change-request gate (#831)
- Differential-check the weather disaster gates against C++ (#797)
- Differential-check the SafeRandom team reservoir bounds (#817)
- Pin cursor atlas indices to the C++ constants (#837)
- Differential-check the blast selection chain against C++ (#790)
- Pin that a rebound pause key replaces the default (#832)
- Pin that F9 capture ignores the window scale (#830)
- Pin that a removed Queron crew member still relaunches (#827)
- Differential-check the melee and teamwork team assignment arms (#826)
- Pin that GVTY hands off to a new attached carrier (#824)
- Pin the hardcoded US language fallback and its error arm (#823)
- Pin that MCOK kills live prey and outlives its own cast (#822)
- Pin that the no-magic-energy rule gates MMED (#820)
- Pin that each recovery elixir installs its own cure (#818)
- Pin how the MGRP combo sets its replica count (#816)
- Differential-check the liquid entry splash against C++ (#785)
- Pin the MSSH stone shield rock combo strength (#793)
- Pin that MARK swaps its projectile for a combo arrow (#814)
- Differential-check the deterministic smallest-team scan bounds (#813)
- Pin that MGBW loads and spends a carried arrow (#812)
- Pin what a released port build reads from this build's wire (#811)
- Pin the MFWL firewall owner and controller split (#810)
- Pin that every MFSK snake chases the selected victim (#809)
- Pin the MBOT blackout rock combo as an exact doubling (#806)
- Drive the shipped MBLS bloodsucker through its aimer (#804)
- Drive the shipped RUND raise undead through its selector (#792)
- Pin the GZ9Z gold combo short-circuit (#803)
- Pin how FHSK picks its fishskin revaluation target (#801)
- Separate volcano advance cost from the dirty-rect scan (#799)
- Pin the WOLI walk-on-liquid duration on its real caster (#798)
- Differential-check the mass-mover slot scan against C++ (#781)
- Drive the shipped MGWP warp through real player controls (#776)
- Drive the shipped EXTG extinguish through real player controls (#782)
- Drive the shipped DGCL dragon call against a real dragon (#789)
- Baseline the per-glyph draw and binding cost of native text (#784)
- Differential-check the poof reaction's unsynchronised draws (#778)

## [0.17.0] - 2026-08-20

### Documentation

- Record the snapshot materiality threshold before optimising it (#756)

### Features

- Offer a gather order only for items a clonk can fetch and carry home (#770)
- Erase voice media key material when a route ends (#768)
- Make the toolbox material and texture selectors pop-up combos (#760)
- Render the toolbox preview as the patterned disc C++ draws (#752)
- Give the component editor selection, clipboard, undo and a measured caret (#750)

### Performance

- Route eligible non-object sprites onto the compact instance (#777)
- Fold the monitor gamma resolve into the presentation draw (#762)
- Lower a rotated definition particle to a compact sprite instance (#761)
- Read one effect per checker instead of copying the whole list (#751)
- Test effect priorities without copying the effect stack (#748)

### Refactoring

- Remove residual unused code (#795)
- Remove obsolete code and consolidate test fixtures (#794)

### Testing

- Drive the shipped MFFS force field through real player controls (#779)
- Drive the shipped Alchemy curse family through its selector (#788)
- Pin that split screens do not multiply the fog cell budget (#787)
- Drive the shipped ETFL eternal flame through real player controls (#786)
- Drive the shipped MDBT firebreath through real player controls (#775)
- Pin that shipped scenarios never animate the sky lighting factor (#780)
- Differential-check the PXS slot allocator against C++ (#774)
- Drive the shipped MFRB fireball through real player controls (#773)
- Drive the shipped MGHL heal through real player controls (#771)
- Pin the fog quad diagonal a shader lookup must reproduce (#769)
- Pin that no shader reading mip-capable art selects a level (#766)
- Pin the 32-bit envelope the fog falloff is exact in (#767)
- Pin that modulate rounds ties away from zero (#765)
- Pin what a string of distinct native glyphs costs today (#764)
- Pin that shipped scenarios never reach the column landscape fallback (#763)
- Pin that every live newgfx particle reaches the snapshot (#758)
- Split the SimulationSnapshot projection by section (#757)
- Pin that a machine with neither presentation path is told it was both (#755)
- Trace identifier lookups over a scenario activation (#754)
- Pin that a group outside the stock sort table packs in insertion order (#753)
- Pin that a retained snapshot survives the engine advancing past it (#747)

## [0.16.0] - 2026-08-19

### Bug fixes

- Resolve monitor gamma into the CPU-path savegame thumbnail (#717)
- Say why the headed GPU probe cannot validate software presentation (#716)
- Report a below-floor adapter as such when falling back to software (#715)
- Deliver the macOS keys SDL names but winit leaves unidentified (#696)
- Size point and line rasters from the world zoom as well as the scale (#685)
- Encode the JIS keyboard keys as SDL's international scancodes (#677)
- Find a moved bundle's interrupted update through its install identity (#671)
- Enumerate multicast interfaces on Windows for LAN discovery (#663)
- Encode macOS F13-F15 as SDL's Cocoa scancodes (#664)
- Send the league End when macOS terminates the app (#662)
- Stop packaging the content repository's own infrastructure (#661)
- Keep repeated component IDs as independent C4IDList entries (#658)
- Split player-file basenames on backslash only where C++ does (#655)
- Bind update recovery to the install identity rather than its pathname (#650)
- Match ControlConfigArea's gamepad claim, key capture and config defaults (#649)
- Truncate a function whose hard inherited has no overload target (#648)

### Documentation

- Record the sampling profile that answers the effect dispatch question (#704)
- Withdraw the effect dispatch timing attribution that did not reproduce (#702)
- Record why the compiled prelude keeps resolving its call sites (#698)
- Record the two accepted keyboard-identity gaps beside their encoders (#694)
- Stop the Pi 0-3 row from ruling out a wgpu-independent presenter (#691)
- Record which call paths the measured lookups come from (#684)
- Link the graphics support matrix and name the surface-lost variant exactly (#682)
- Drop the references to the deleted third-party content document (#657)

### Features

- Run the interactive window without a GPU adapter (#714)
- Choose between GPU and software presentation from the graphics floor (#710)
- Present the CPU frame through a wgpu-free window surface (#708)
- Composite the CPU frame for a presenter with no GPU adapter (#706)
- Compose with an IME in the scenario search field (#687)
- Fall back to a software adapter when no hardware one answers (#686)
- Show IME composition in the classic edit and place the candidate window (#681)
- Declare and document the interactive graphics floor (#674)
- Add the symbolised stack walk and loaded-module list to the Windows crash report (#673)
- Count what effect dispatch materialises per tick (#672)
- Log the C4AulScriptEngine link summary (#668)
- Report every unmet graphics requirement in one diagnostic (#665)

### Performance

- Recompose only the landscape region an edit dirtied (#707)
- Keep the map planes when only the material catalogue changed (#705)
- Upload a landscape edit's own rectangle instead of its whole rows (#701)
- Resolve a call's host callee once instead of once per argument (#697)
- Test a call's name before walking the host tables for its reference-ness (#695)
- Stop resolving a call's name twice to answer one question (#693)
- Resolve a callback before materialising its calling context (#680)
- Retain the shader-landscape composition resources across updates (#669)
- Read back only the thumbnail for a thumbnail-only save (#670)
- Stop allocating an effect callback name per dispatch (#667)

### Refactoring

- Route the CPU presentation branch through a target it does not own (#713)
- Let the shell window hold no retained GPU renderer (#712)
- Add an ordered C4IDList component list type (#656)

### Testing

- Pin that an oversized landscape degrades to CPU presentation (#711)
- Pin the landscape scissor at a detail factor above one (#709)
- Pin which retained landscape resources each change invalidates (#703)
- Measure what an effect-heavy tick materialises around callbacks (#700)
- Pin that an unchanged shader landscape uploads nothing (#699)
- Report what a PXS execute pass walks against what it finds (#690)
- Attribute C4Script identifier lookups to their call path (#689)
- Bench the snapshot projection against the advance it follows (#688)
- Differential-check the savegame association pass loop (#679)
- Measure C4Script identifier lookups by family (#678)
- Export component order from the C++ bridge and compare it field-wise (#676)
- Pin that a denied effect stop keeps its slot (#666)
- Differential-check the four savegame player matching passes (#653)
- Pin every tutorial landscape against the C++ Surface8 oracle (#652)
- Cover a headless host preparing its masterserver registration (#651)

## [0.15.0] - 2026-08-18

### Bug fixes

- Refuse a global func that names a declaring script's local (#646)
- Flush the deferred participant list in its quoted native form (#645)
- Do not draw the refused league registration notice on a headless server (#642)
- Quit when a command-line start has no startup generation behind it (#635)
- Defer the runtime config writes C++ leaves to its shutdown save (#633)

### Features

- Record component archive ownership so a retired pack can be removed (#643)
- Silence a muted participant's voice from the runtime client list (#640)
- Define the observer and multiple-local-player voice source policy (#639)
- Tell a client when the async deadline discarded its control (#638)
- Park a headless server for the next console command after a round (#634)

### Performance

- Move the definition and sector lookup tables onto the engine hasher (#637)

### Testing

- Record the headless determination for the console-mode gates (#632)
- Pin the sailboat hull solid mask across ChangeDef (#630)

## [0.14.0] - 2026-08-17

### Bug fixes

- Preserve pre-join player source handling (#622)
- Invalidate explicit script-menu rows on refill (#621)
- Preserve oversized script literals in unary expressions (#619)
- Persist in-game display settings (#613)
- Report particle reload I/O failures synchronously (#610)
- Remove unsupported save shortcuts (#615)
- Isolate the Rust debug HUD from parity runs (#620)
- Localize runtime client information dialogs (#618)
- Localize remaining startup options strings (#616)

### Features

- Render definition icons in object list (#623)

### Refactoring

- Unify definition shape refresh (#612)
- Remove unused console completion helper (#609)

### Testing

- Isolate congested UDP peer sends (#617)
- Pin voice replay ownership (#614)
- Pin one-shot second timer backlog behavior (#611)

## [0.13.3] - 2026-08-16

### Bug fixes

- Align dialog hit testing with resolved icons (#594)
- Resolve the native League evaluation icon (#603)
- Route native network key callbacks (#604)
- Replay intra-frame ActMap sound transitions (#605)
- Cfg-gate Unix-only client mesh import (#592)
- Render action overlays from row zero (#595)
- Mirror raw rotation predicate in face blits (#598)
- Log savegame player removal messages (#593)
- Drain pending stream before headless round exit (#600)
- Localize definition overload diagnostics (#597)
- Snapshot native object menu pictures (#599)
- Default unmanifested float actions to native bounds (#601)

### Testing

- Restore About list scrolling coverage (#596)

## [0.13.2] - 2026-08-16

### Testing

- Cover the Queron assassin relaunch cycle (#591)

## [0.13.1] - 2026-08-16

### Bug fixes

- Honor native zero float defaults for resources (#506)
- Distinguish missing and explicit runtime player indexes (#501)
- Process c4group update commands in sequence (#500)
- Deactivate remote clients after player elimination (#507)
- Gate port capabilities for stock cpp peers (#502)

### Performance

- Reduce volcano landscape upload churn (#504)
- Skip unchanged sector rank refreshes (#503)

### Testing

- Cover runtime network join through running state (#505)

## [0.13.0] - 2026-08-15

### Bug fixes

- Preserve shader landscape output across frames (#490)
- Honor PXSGfx particle rendering option (#489)
- Allow inactive network clients to re-add players (#488)
- Add adjustable voice playback boost (#484)
- Broadcast retired profiles to network clients (#483)

### Features

- Support voice chat in network lobbies (#487)

### Performance

- Compact owner-color object passes (#486)

## [0.12.1] - 2026-08-14

### Bug fixes

- Reconcile retired player profiles before rejoin (#479)
- Improve proximity voice reliability and quality (#478)
- Keep connected clients in the lobby after restart (#476)
- Dispatch ExecuteCommand Call before later script mutations (#462)
- Attach Push, Pull and Fight grounding after their live SetDir (#461)
- Clear removed object references from retained VM temporaries (#463)
- Preserve native C4ValueHash state when object references are cleared (#464)

### Performance

- Keep slowest tests below ten seconds (#477)
- Instance retained landscape fog chunks compactly (#473)

## [0.12.0] - 2026-08-14

### Bug fixes

- Correct the false port-only packet-ID rationale in capabilities.rs (#458)

### Continuous integration

- Run workspace quality checks on pull requests (#451)

### Features

- Encrypt the voice media lane under a per-route key exchange (#465)
- Add echo, noise and gain processing to voice capture (#466)
- Buffer and conceal remote voice playback (#460)
- Give the voice mix headroom for multiple simultaneous speakers (#459)
- Put voice activation on the Options Audio sheet (#456)
- Expose voice chat settings on the Options Audio sheet (#454)
- Add voice activation as an alternative to push-to-talk (#453)

### Performance

- Measure retained renderer stages (#455)

### Testing

- Record Gold Rush parity through frame 15000 (#450)
- Gate headed GPU surface teardown (#449)

## [0.11.2] - 2026-08-14

### Bug fixes

- Preserve host geometry across SetGraphics (#447)
- Reject runtime joins past the player limit (#446)
- Preserve native action procedure direction ordering (#444)
- Settle small earthquake debris (#440)
- Resynchronize fixed position after cross-check fling (#439)
- Reject exact saves with missing player state (#437)
- Clear removed object references before arrow calls (#436)

### Testing

- Restore tutorial 04 through 07 virtual routes (#441)

## [0.11.1] - 2026-08-13

### Bug fixes

- Attribute host waits before sizing PreSend (#431)
- Accept complete control packets on the host (#427)

### Continuous integration

- Isolate merge-queue shards from diagnostics (#430)
- Pin the NSIS distribution download (#428)

### Documentation

- Retire PORT_STATUS.md into issues and inline its rationales (#416)

### Testing

- Synchronize dual-route reconnect coverage (#429)

## [0.11.0] - 2026-08-13

### Bug fixes

- Keep a client runtime status barrier equal to the status its network layer holds (#341)
- Preserve effect callback host state (#339)
- Restore Eke airbike dismount on quick landing (#337)

### Features

- Fly the Eke airbike twice as fast (#340)
- Add proximity voice chat (#338)

### Refactoring

- Own the window surface and drop the vendored pixels fork (#342)

## [0.10.1] - 2026-08-12

### Bug fixes

- Restore two diagnostics that log filters were swallowing (#332)
- Quiet the calloop stale-source line without a winit fork (#329)
- Reuse the Wayland key-repeat timer (#327)
- Keep unassociated admission failures out of the lobby log (#326)
- Mute calloop stale-source warnings on Wayland (#325)
- Discover FluidSynth on Linux like libxmp (#324)

### Features

- Fly the Eke airbike hold-to-steer and fix two control-chain parity defects (#335)
- Find new network games without a manual refresh (#323)
- Wire object placement, the object list and the component editors (#313)

### Performance

- Improve Raspberry Pi frame throughput (#320)

### Refactoring

- Consolidate repeated test infrastructure (#331)
- Consolidate repeated test infrastructure (#328)

### Testing

- Pin that the no-friendly-fire rule cannot stop blast damage (#333)
- Pin the rule chooser that creates Hazard's no-friendly-fire rule (#322)
- Pin the team alliance and rule gates Hazard hit checks read (#321)

## [0.10.0] - 2026-08-12

### Continuous integration

- Shard Rust coverage collection (#309)
- Install a prebuilt git-cliff so release preparation meets its SLO (#304)

### Features

- Wire the console viewport context menu and developer toolbox (#307)
- Restart a network round without dropping the session (#305)

## [0.9.8] - 2026-08-11

### Bug fixes

- Draw a classic menu row whose picture never resolved (#295)
- Rebuild the fog repeller list when fog of war is enabled (#280)

### Continuous integration

- Auto-merge renovate dependency updates through the merge queue (#263)

### Features

- Let a host bar rejoining after elimination (#302)
- Lift dragon rock shadows at the edge of their own darkness (#276)
- Pick the X11 backend when Steam Input drives a Wayland session (#266)
- Show time of day with a sun and moon (#264)

### Performance

- Instance exact old-style PXS line fragments (#281)
- Coalesce compatible fogged landscape draws (#275)
- Retain and instance object sprites (#265)

### Refactoring

- Remove obsolete internal code (#278)

### Testing

- Pin network-host readmission of a retired profile (#279)

## [0.9.7] - 2026-08-11

### Bug fixes

- Keep the Eke remote control from walking its pilot (#258)
- Return a failed network host start to the startup dialog with its error log (#257)
- Give a network host's pre-publication load its scenario folder materials (#255)
- Name the game a network join is connecting to instead of its addresses (#249)

### Performance

- Sustain native cadence with 1000 Stippels (#259)
- Skip the discarded solid mask scan in grid worlds (#250)

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
