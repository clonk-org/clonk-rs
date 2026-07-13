# C++ Menu Parity Inventory

This is the recursive completion checklist for the Rust menu port. The C++
classes in `../src/` are authoritative. A top-level screen is not complete
until every child sheet, context menu, confirmation/input dialog, callback,
scroll path, and transition reachable from it is complete.

Status meanings (the status applies only to the bounded row, not its parent
screen or an unwired library component):

- **Complete**: the reachable Rust path covers the C++ behavior and
  presentation with executable evidence and has no generic fallback.
- **Partial**: a classic-shaped implementation exists, but listed behavior is
  missing or indirect.
- **Fail-fast**: Rust logs and returns an error instead of drawing a generic or
  incomplete pane. This is preferable to false parity, but is not completion.
- **Missing**: no usable classic Rust implementation exists. A reachable
  generic, synthetic, status-only, or externally delegated substitute is still
  Missing, not Partial or Fail-fast.

The C++ source and shipped `content/`/`planet/` data are read-only oracle
material for this inventory. Implementations belong under `rust/`; never edit
the oracle to make a Rust result appear correct.

## Recursive census

- Startup graph: `C4Startup::SwitchDialog` plus every callback/subdialog from
  `C4StartupMainDlg`, `C4StartupScenSelDlg`, `C4StartupNetDlg`,
  `C4StartupPlrSelDlg`, `C4StartupOptionsDlg`, and `C4StartupAboutDlg`.
- In-game graph: `C4MainMenu`, `C4Menu`, `C4ObjectMenu`, player team selection,
  object context activation, and generic `C4GUI` dialogs.
- Overlay graph: `C4GraphicsSystem`, `C4UpperBoard`, `C4MessageBoard`, every
  `C4Viewport::DrawOverlay` child, `C4MouseControl::Draw`, network/debug status,
  screenshot feedback and eliminated/observer states. Non-dialog presentation
  is part of the literal recursive screen census.
- Rust-only graph: every app `StartupView`, retained generic renderer,
  status-state producer, environment/developer route and empty-state fallback
  must either be explicitly isolated or return a logged error before drawing.
- Script graph: 147 `CreateMenu` calls in 109 shipped `content/**/*.c` and
  `planet/**/*.c` files, plus 401 `AddMenuItem` calls. Styles comprise Normal,
  Context, Info, and Dialog.
  Full coverage comes from implementing the complete CreateMenu/AddMenuItem
  grammar rather than hardcoding individual scenario screens.
- Modal graph: at least 134 C++ message/confirmation/input/remove-dialog call
  sites across startup, networking, options, league, player, scenario, and
  update flows.
- Editor-only GUI is tracked separately at the end; it is never silently
  omitted from the literal full-GUI scope.

## Startup root

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Main six-button screen | `C4StartupMainDlg.cpp:38-122`; `rust/crates/lc-frontend/src/startup_main_menu.rs` | **Partial** | The classic path has strong pixel/input coverage and the app now preflights exact images/fonts before rendering. Fade/back-stack and common-screen behavior still prevent completion. |
| Eager startup graphics/font bootstrap | `C4Startup::EnsureLoaded`, `C4StartupGraphics::Init`/`InitFonts` | **Fail-fast** | One deterministic aggregate preflight now requires the oracle-ordered 16-image bundle (including the recursively used `StartupPlrPropBG.png` and `StartupPlrCtrlType.png`) and Caption/Main/Title/MainSmall shadowless RX fonts before view/model/child/status checks, cache lookup, or logical/native pixels. All seven Rust `StartupView` roots are covered; `NetworkLobby` retains the prerequisite because C++ reaches its lobby before `C4Startup::Unload`, although it is not a startup `DialogID`. Decoded surfaces/fonts are checked for basic usability without inventing atlas dimensions C++ never validates. At integer application scale above one, Main also refuses a missing or mismatched scale-native font atlas instead of silently drawing logical text. Rust still binds the shipped `.png` entries and Endeavour-derived font builders; C++ stem lookup through group overrides and configured/system RX faces remains to port. Global `GUIBigArrows`/`GUISpinBoxArrow` validation remains outside this startup-specific bundle slice. |
| Main screen fixed-default resource failure boundary | `C4StartupMainDlg`; startup graphics resources | **Fail-fast** | For the currently supported shipped-resource path, the app logs the complete missing set and returns an error before bitmap-font, optional-image, blue-pane or solid-button substitutes can become player-visible. It still hard-requires `LoaderGoldmine1.png` and Endeavour-derived fonts; C++ loader fallback and configured/system font resolution are the gap recorded above. |
| Dialog switch/fade/back stack | `C4Startup::SwitchDialog`, `C4StartupDlg::OnClosed` | **Partial** | Match fade timing, previous-dialog ownership and every Escape/back transition; Rust currently switches a flat `StartupView`. |
| Common startup cursor/focus/hotkeys/tooltips | `C4GUI::Screen`, `Dialog`, `Control` | **Partial** | Bottom-button and Options-tab pointer presses now preserve prior focus where `IsFocusOnClick=false`, and source-aware gamepad horizontal routing no longer impersonates keyboard Back/Crew. That bounded handler only removes the false actions; it does not claim complete C++ traversal. Centralize the classic cursor, 500 ms tooltip delay, full Tab/Shift-Tab traversal, About gamepad traversal, Options Ctrl+Tab and About/Network Alt mnemonics instead of per-screen approximations. |
| Participants Add/Remove context and player submenu | `C4StartupMainDlg::OnPlayerSelContext*` | **Partial** | Exact autosized markup label target and tooltip, recursive lazy Add/Remove tree (including empty 40x7 children), raw filesystem/config ordering, raw Remove indices, player icons/tooltips, activation/validation/deduplication and focus suppression are covered. Missing application paths and configuration-write failures retain diagnostic status but now hit the centralized typed boundary before startup pixels or cache output; their classic error dialogs remain unported. |
| Rust startup model/child/status boundary | no C++ screen; `GameApp::preflight_startup_presentation` | **Fail-fast** | After the eager bootstrap passes, both logical and scale-native presentation reject a missing required controller, then any active unported child, then generic diagnostic status before cache replay or pixel composition. Typed child payloads cover all five non-Program Options sheets, About Licenses and Network Chat; a stale matching cache cannot reveal the prior supported pane. This is a refusal boundary, not implementation of any listed descendant. |
| Startup base-frame cache and nested overlays | C++ redraw/invalidation ownership; `GameApp::render_for_presentation` | **Partial** | Context menus, Password/Comment input, definition selector, message dialogs and the participant tooltip are excluded by one shared cache-eligibility predicate for both lookup and write. Thus composited overlay pixels never replace the reusable base frame, including same-version open/render/close cycles. A stale game-over dialog is no longer treated as a startup overlay at all: the lifecycle boundary wins before cache lookup. Broader classic dirty-rectangle/redraw ownership remains unported. |
| First-run new-player properties modal | `C4StartupMainDlg::OnShown` | **Missing** | Depends on player-properties dialog. |
| Automatic/manual/incoming update flow | `C4StartupMainDlg::OnShown`, `C4UpdateDlg` | **Missing** | See update subtree. |
| F6 editor launch from startup | `C4StartupMainDlg::SwitchToEditor` | **Missing** | Bind and validate the legacy editor launch. |
| Rust editor scenario/controller route | no C++ startup-list equivalent | **Fail-fast** | `ScenarioKind::Editor`, touch activation and `StartupMenuAction::EditEntry` return typed, logged boundaries; the external-editor substitute was removed. Keep the guard until the real developer UI exists. |
| Startup fatal/restart log info dialog | `C4Startup::DoStartup` | **Missing** | Classic expandable info dialog. |

## Scenario selection subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Local scenario book chrome/list/info/buttons | `C4StartupScenSelDlg`; `startup_scensel.rs::ScenselDialogFocus` | **Partial** | Exact focus chrome/order, basic keyboard/gamepad traversal, disabled-control skipping and pointer/touch hit bounds are covered. The exact F5 Refresh, F2 Rename, Delete and Alt+M Mission Access actions remain unported; their dialog/list-visible priority is enforced and each available callback now logs and fails typed before a lower-priority control can impersonate it. |
| Recursive `.c4s`/`.c4f`/directory discovery | `C4ScenarioListLoader` | **Complete** | Retain recursive discovery/order tests. |
| Recursive search results and deep-folder Back reconstruction | C++ current-folder filter: `C4StartupScenSelDlg::UpdateList`; requested Rust extension: `collect_frontend_search_matches`, `Folder::Start`, `FolderBack` | **Partial** | Rust intentionally implements the user-requested recursive search extension: it walks every descendant and reconstructs every intermediate folder layer so Back pops one level at a time. C++ filters only the immediate current folder, so this must remain documented as a deliberate product divergence rather than exact oracle parity. Keep deep-nesting and duplicate-path regressions. |
| Folder navigation and folder metadata | `Folder::Start`, `FolderBack` | **Partial** | Runtime reload/mutation parity. |
| Asynchronous scenario-loader progress state | `C4StartupScenSelDlg::UpdateList`, `C4ScenarioListLoader::GetProgressPercent` | **Missing** | C++ clears the list and shows the localized loading percentage while recursive discovery is active. Rust has no corresponding child state, progress label, cancellation/refresh ownership or transition back into the populated book. |
| Folder-map view (`FolderMap.txt`) | `C4StartupScenSelDlg.cpp:47-404,1016-1023`, `C4MapFolderData`; shipped `content/Western.c4f/FolderMap.txt` | **Fail-fast** | Direct opens, recursive search/start, packed logical groups and duplicate merged roots inspect every contiguous `.c4f` ancestor/contributor case-insensitively and return a typed, logged boundary before the ordinary book can draw. The actual map background, buttons, overlays, access graphics and info pane remain unported. |
| Search edit/filter | `OnSearchBarEnter`, `UpdateList`, `KeySearch`, `C4GUI::Edit` | **Partial** | Submit-only markup-stripped filtering, Ctrl+F select-all, caret/selection, word edits, clipboard shortcuts, mouse capture/double-click, horizontal scroll, blink, and render tests work. The exact state-dependent Cut/Copy/Paste/Clear/Select-all classic popup, right-down/Apps-key triggers, retained logical focus with suppressed focus drawing, clipboard mutation rules, and activation-release capture are covered. Remaining: non-Windows middle-click primary selection and the exact zoomed `¦` caret glyph. |
| Description `TextWindow` scrolling | `C4GUI::TextWindow`, `ScrollWindow` | **Complete** | Wheel, clipping, fixed pin, track jump-and-drag with capture, and held arrows match the C++ geometry/conversions. |
| Scenario list scrolling | `C4GUI::ListBox` | **Complete** | Selection-follow viewport, wheel, clipping, fixed pin, scrolled clicks, captured track drag, held arrows, end-stopping Up/Down, and fully-visible-row PageUp/PageDown/Home/End are covered. |
| Choose Definitions checkbox and definition selector | `StartScenario`, `C4DefinitionSelDlg`; `lc-app/src/main.rs::PendingDefinitionSelection` | **Complete** | Selection/reset rules, LocalOnly, recursive focus/input, raw-order `*.c4d`, fixed/optional checks, exact dialog/list/preview/buttons, scrolling/title drag, F5 rebuild, nested error, modal/capture cleanup, cancel retention, ordered output and rooted loading are covered. Local versus `NetworkHost` mode survives refresh, cancel, error and accept. |
| Scenario rename | `ScenListItem::KeyRename`, `Entry::RenameTo` | **Missing** | F2 now fails typed when a row is selected. Inline edit and all failure dialogs remain. |
| Scenario delete | `KeyDelete`, `DeleteConfirm` | **Missing** | Unmodified Delete now takes the selector's higher priority and fails typed when a row is selected, including while Search has focus; without a selection it correctly falls through to the edit. Original warning, confirmation, deletion and errors remain. |
| Mission access/password | `KeyCheat`, `KeyCheat2` | **Missing** | Exact Alt+M priority is enforced across the visible book and fails typed before the NetworkHost Comment mnemonic. The input modal and module add/remove remain. |
| Start validation/warnings | `Scenario::CanOpen`, `DoOK` | **Missing** | Access, replay/network, player limits, dedicated warnings. |
| Local Fair Crew/Record option buttons | `C4GameOptionButtons`; `lc-frontend/src/game_option_buttons.rs` | **Partial** | Exact strip geometry/resources, tooltips, focus, keyboard/pointer/touch/gamepad input, normal config persistence and `ForcedNoCrew` constraints are app-wired. Persistence failures retain diagnostics and fail at the centralized startup-status boundary before presentation; the classic error path remains unported. |
| Network scenario book | `C4StartupScenSelDlg(true)`; `ScenarioSelectorMode::NetworkHost` | **Partial** | Network Create Game opens the selector before binding a socket. Recursive focus, touch search/list/open/back, modal/context ownership, definition-return state and close-surviving input capture are covered within the shared book. Selector shortcut priority is now fail-closed; FolderMap, the actual rename/delete/access/refresh actions and validation remain. |
| Network selector option buttons | `C4GameOptionButtons`; `lc-frontend/src/game_option_buttons.rs` | **Partial** | Internet, League, Password, Comment, Fair Crew and Record use classic layouts/states/tooltips and keyboard, pointer, touch and gamepad routing. Boundary pass-through, per-gesture capture and resize cancellation are tested; normal values load/persist through config. Alt+M now reaches the higher-priority selector Mission Access boundary instead of opening Comment, and an open context menu suppresses the underlying option strip. This row remains scoped to the selector, not the lobby. |
| Password/Comment `InputDialog` from network selector | `C4GameOptionButtons::OnBtnPassword`, `OnBtnComment`; `lc-frontend/src/input_dialog.rs` | **Partial** | Both selector callers use the resource-validating classic modal with edit/caret/selection/context actions, max length and all input modes. Strict underlying-screen exclusion, Apps-key context ownership, per-button gesture capture, close-surviving release consumption and resize cancellation are tested. Persistence failures now take the typed startup-status boundary before pixels; their classic error modal and other `C4GUI::InputDialog` consumers remain Partial below. |

## Startup network browser and IRC subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Network browser chrome | `C4StartupNetDlg` | **Partial** | Static classic shell/controller exists. |
| Retained NetDlg Internet refresh | `C4StartupNetDlg::OnShown`, `UpdateMasterserver`; `NetDlgController::sync_masterserver_signup_from_config` | **Partial** | Returning from the host selector updates the Internet icon/config and deterministically renders or removes the initial masterserver query row in place while retaining dialog state. Live query/status transitions and spawned game-reference rows remain unported. |
| Live game list/query entries | `C4StartupNetListEntry`, `UpdateList` | **Missing** | Masterserver/LAN/direct queries, references, status, errors, refresh throttling. |
| Reload and direct-address two-stage query/join | `C4StartupNetDlg::DoRefresh`, `DoOK` | **Fail-fast** | Reload returns a typed action boundary before generic status; this conservatively overblocks even C++'s within-one-second rate-limited no-op. A nonempty raw edit value is considered direct only while the edit retains focus; the value is not trimmed. The app then returns a typed action boundary before spawning network work or setting status. Implement the direct-reference query row, second Enter/join, throttling and errors. |
| Join validation/redirect/discovery modals | `DoOK`, `DoRefresh` | **Partial** | Empty selection now opens the exact classic `Cannot join game` OK/Error dialog. Bad references, version checks, redirect and discovery/error paths remain. |
| Create Game transition | `C4StartupNetDlg::CreateGame`; `lc-app/src/main.rs::process_network_dialog_actions` | **Complete** | It opens `C4StartupScenSelDlg(true)`/`NetworkHost` before any host socket or `NetworkManager` exists. |
| Selected host scenario/definitions staging | `C4StartupScenSelDlg::StartScenario`; `StagedNetworkHostScenario` | **Partial** | Scenario, ordered definitions, immutable accepted options, scale-1 US loader/resources, effective `Config.Network.LocalName`/hostname identity and the bounded lobby model are validated before asynchronous host activation. Once accepted, the transition is noninteractive across keyboard, pointer, touch, gamepad and programmatic actions. Save/replay, league, embedded clients, raw startup participants, script-player capacity, noncanonical names and scale-native lobby text are typed pre-bind refusals; broader start validation remains. |
| Post-connect lobby boundary | `C4Game::NetworkJoin`, `C4GameLobby`; `GameApp::poll_startup_network_connection` | **Partial** | A staged ordinary host enters the exact Players-sheet lobby over the scenario loader background while retaining its manager. Every unsupported child/network action is typed fail-fast and never renders the generic pane. Join and unstaged-host completion still tear down and retain the refusal detail before presentation. |
| IRC login sheet | `C4ChatControl` | **Fail-fast** | Entering Chat returns a typed `NetworkGameChat` boundary before the empty Rust box or cached GameList can render. Games/Back reconstruction remains live. Implement nick/password/real-name/channel/connect, disclaimer and login errors. |
| IRC server/channel/query tabs | `C4ChatDlg`, `C4ChatControl` | **Missing** | Logs, input/history, nick lists, unread/query state. |
| IRC command language and quit confirmation | `C4ChatControl::ProcessInput` | **Missing** | Full command dispatch and confirmation. |

## Startup player subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player list/activation/info/portrait | `C4StartupPlrSelDlg` | **Partial** | Classic visual/controller and activation persistence exist. |
| Player row Properties/Delete context | `PlayerListItem::ContextMenu` | **Partial** | Whole-row right-down now opens the exact two-entry classic popup with no initial selection, retained keyboard focus, focus suppression, tooltips, pointer/keyboard/touch/gamepad routing, outside-down pass-through and Delete activation into the exact confirmation chain. Properties preserves classic popup close/Click ordering and then returns the indexed typed action boundary without status or domain mutation. |
| New Player / Properties form | `C4StartupPlrPropertiesDlg` | **Fail-fast** | New, double-click/F2/button Properties and row-context Properties return indexed typed action boundaries immediately. Implement name, colors, control set, mouse, movement mode, portrait selection and validation/save-error descendants. |
| Portrait selector | `C4PortraitSelDlg` | **Missing** | Location combo, image grid, flags, OK/Cancel. |
| Crew mode and crew detail list | `SetCrewMode`, crew list classes | **Fail-fast** | The indexed Crew action returns a typed boundary before status or mutation. This deliberately overblocks cases where C++ would merely fail to open or show its no-crew response until that exact child is ported. Implement participation, stats, portraits and sorting, then recurse through rename/delete/death-message input/context and their validation/error dialogs. |
| Crew Rename/Delete/Death Message context | crew item callbacks | **Missing** | Inline rename, input modal, confirmation/errors. |
| Player/crew delete and validation dialogs | `OnDelBtn`, property close paths | **Partial** | Player Delete button/key/context now use the exact Yes/No warning (including the strict >10-hour suffix), permanently remove packed or directory `.c4p` groups, always rebuild list/selection/Participants, and show the exact failure dialog. Crew deletion and property validation remain. |

## Startup options subtree

Only the Program sheet may currently render. Other sheets return a logged
error instead of showing blank or generic panes.

| Sheet or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Options chrome/tabs | `C4StartupOptionsDlg` | **Partial** | Layout, pointer sheet switching, focus-preserving pointer clicks and typed unsupported-sheet entry are covered. Gamepad traversal between Back and the tabular is exact; attempts to traverse forward into Language or backward-wrap into Advanced return typed action boundaries without moving focus. Ctrl+Tab and full Program-child focus traversal remain missing. |
| Program | program sheet controls | **Partial** | Timestamp is live; language/font/chat/preload/fair-crew/reset/advanced are inert or missing. |
| Graphics | graphics sheet controls | **Fail-fast** | Typed entry boundary precedes cache/pixels. Implement display-mode combo/recreate, scale spin/slider and Test's timed Yes/No/revert dialog, five toggles, smoke slider and fire controls. |
| Sound | sound sheet controls | **Fail-fast** | Typed entry boundary precedes cache/pixels. Implement front-end/game music and sound checks, volume sliders/test sound and the higher-priority F3 toggle. |
| Keyboard | `ControlConfigArea` | **Fail-fast** | Typed entry boundary precedes cache/pixels; the generic `ControlOptionsState` input/render/persistence route is disconnected, so caught failures cannot reset/rebind/save. Implement control-set selector, twelve keys, reset and KeySelDialog. |
| Key capture modal | `KeySelDialog` | **Missing** | Classic modal and device input semantics. |
| Gamepad | `ControlConfigArea` | **Fail-fast** | Typed entry boundary precedes cache/pixels. Implement device/control-set selectors, selected-device filtering/opener, twelve bindings, GUI-control recreation and reset. |
| Network | network sheet controls | **Fail-fast** | Typed entry boundary precedes cache/pixels. Implement TCP/UDP/reference/discovery enable/spin controls, alternate-server warning/don't-show-again, automatic update, UPnP, machine/nick edits, port-collision dialogs and SaveConfig persistence. |
| Reset confirmation/quit | `OnResetConfigBtn` | **Missing** | Confirmation and application exit. |
| Advanced warning/config editor | `C4StartupOptionsAdvancedConfigDialog` | **Missing** | Dynamic section tabs and typed controls. |
| Timed scale confirmation | `ResChangeConfirmDlg` | **Missing** | 12-second Yes/No and automatic restore. |
| Validation/restart/font errors | options callbacks | **Missing** | Classic messages. |

## About/update subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Credits page/chrome | `C4StartupAboutDlg` | **Partial** | Strong static render; developer TextWindow scrolling is missing. |
| Licenses list/text page | `LicenseWindow` | **Fail-fast** | Typed entry boundary precedes cache/pixels while preserving button-Back to Credits and dialog Back to Main. Implement the license list, selected title/body TextWindow and both scroll paths. |
| Update button and update controller | `C4UpdateDlg` | **Fail-fast** | Check Updates returns a typed action boundary immediately, without generic status. Implement lookup/wait/Abort, HTTP/version error, redirect Yes/No, available/no-update/error, DownloadDlg progress/cancel/save/error and apply-process InfoDialog/log transitions. |

## Network start/lobby subtree

The app deliberately rejects its old generic lobby after host/join connection.
`lc-frontend/src/game_lobby.rs` is a resource-validating implementation of the
initial Players-sheet slice. `lc-app` now owns that bounded slice only for a
staged ordinary host; every unsupported recursive child and all other entry
paths remain typed fail-fast. Component-only rows below are therefore Partial,
never full app-level completion.

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Initial network start wait | `C4Network2Dialogs.cpp:114-173,552-586`, `C4Network2StartWaitDlg` | **Missing** | Joining-client list/status, host-side remote Kick rows, Restart/Cancel, and the client-specific abort dialog reached after the lobby. |
| Password challenge and connection progress | `C4Network2Dialogs`, network join callbacks | **Missing** | Password retry/cancel, wait/progress/error and return-to-browser transitions. |
| Host/join app entry | `C4GameLobby::MainDlg`; `GameApp::poll_startup_network_connection` | **Partial** | The staged ordinary host constructs and renders the exact Players-sheet slice; join and unstaged host remain typed fail-fast and tear down the new manager. Unsupported host children also fail at typed boundaries instead of falling through to the generic lobby. |
| Legacy generic `NetworkLobbyState` pane | no C++ visual authority | **Fail-fast** | State may still be constructed internally, but the typed, logged `StartupScreen` preflight rejects `StartupView::NetworkLobby` before cache lookup even with empty status and a matching stale cache. Remove the dormant model when the classic lobby owns the route. |
| Exact lobby frontend resource boundary | `C4GameLobby.cpp:141-314`; `lc-frontend/src/game_lobby.rs::LobbyResources` | **Partial** | The staged host preflights every required classic image sheet/font, including `GUIContext.png` and shadowless `TooltipFont`/BookFont, before binding a socket; missing or invalid resources are typed refusals. Only the initially visible Players slice is app-wired. |
| Scenario/team/parameter metadata projection | `C4Scenario`, `C4TeamList`, `C4GameParameters`, `StdCompilerINIRead`; `lc-engine/src/scenario.rs` | **Partial** | Exact recursive INI hierarchy, case/duplicate/default/string semantics, compound-array recovery, teams, clients, savegame/replay rules and definition-resolution boundaries are projected and tested, including shipped content. Runtime-selected loader resources, current roster/script-player naming, config/network/league adjustments and old-save effective definition resources still require app ownership. |
| `PID_Status` / `PID_StatusAck` barrier foundation | `C4Network2Status`, `C4Network2::ChangeGameStatus`; `StatusBarrier`, `HostHandle`, `ClientHandle`, `NetworkManager` | **Partial** | Exact signed payloads, host barrier transitions, stale/duplicate rejection, `host_change_status`/`host_status_reached`, client acknowledgement, app events and real-TCP tests exist. The unwired lobby does not initiate or consume its status lifecycle. |
| `PID_LobbyCountdown` transport foundation | `C4PacketCountdown`, `Countdown`; `LobbyCountdown`, host/client session APIs | **Partial** | Exact codec, abort value, timer cadence predicate, host-only broadcast, client/app events, identity rules and real-TCP round trips are tested. No app lobby owns the countdown timer or applies the events to UI/start state. |
| `PID_ReadyCheck` / lobby-ready foundation | `C4PacketReadyCheck`; `ReadyCheck`, `HostHandle::set_lobby_ready`, `ClientHandle::set_lobby_ready`, `NetworkManager::set_local_ready` | **Partial** | Request/reply codec, role-correct host/client APIs, origin validation, relay rules, bounded app telemetry and real-TCP tests exist. No app lobby binds them to its checkbox, roster or ready-check modal. |
| Main transparent chrome and responsive layout | `C4GameLobby::MainDlg::MainDlg`; `game_lobby.rs` | **Partial** | The bounded scale-1 host route owns lifecycle/teardown, loader-background composition, responsive layout, exact general/option tooltip fonts, focus and local selection/scroll/option traversal without frame-cache reuse. Recursive sheets, dialogs, chat/edit contexts, network events and side effects remain typed refusals. |
| Chat log, scrolling and input edit | `C4GameLobby.cpp:272-280,475-766,1018-1056` | **Partial** | Component UI, scrolling, focus and edit requests exist; paste/history, synchronized colored log ownership, and lobby-local `/joinplr`, `/plrclr`, `/start`, `/abort`, `/readycheck`, `/help` dispatch are not app-wired. Transport cannot send ordinary chat yet because `CID_Message` is absent. |
| Lobby chat `C4ControlMessage` / `CID_Message` | `C4ControlMessage`, `CID_Message`; `lc-app/src/network.rs` | **Missing** | There is intentionally no opaque placeholder. Implement the conditional private-recipient codec plus authoritative player/team ownership and visibility validation before connecting chat input. |
| Chat input context popup | `C4GUI::Edit::OnContext`; `LobbyChatRequest::OpenContextMenu` | **Partial** | Component emits typed context requests; app must open and dispatch Cut/Copy/Paste/Clear/Select-all using the shared classic menu. |
| Direct lobby Exit | `MainDlg::OnExitBtn`, `OnClosed` | **Complete** | Exit/Escape closes directly with no invented confirmation, drops countdown/lobby/loader/network state and queued synchronization resources, returns to startup Main and restores menu music. |
| Ready checkbox/loading lock | `MainDlg::OnReadyCheck`, `UpdatePreloadingGUIState`, `RequestReadyCheck` | **Partial** | Visual/input state and tested host/client ready APIs exist separately. App wiring to `set_local_ready`/events, authoritative roster state, cooldown, forced reset and preload gating is missing. |
| Host Start/Cancel and synchronized countdown | `MainDlg::OnRunBtn`, `Start`, `OnCountdownPacket`; `Countdown` | **Partial** | Component phases/locking and tested `PID_LobbyCountdown` transport exist separately. Wire validation, timer cadence, broadcast/event application, abort, auto-start, status transition and warnings into the app lobby. |
| Bottom lobby game-option strip | `C4GameOptionButtons`; `game_option_buttons.rs` | **Partial** | Exact Host/Client contexts, focus and countdown locks exist as a reusable component. Lobby Internet, Password, Comment, Fair Crew and Record controls still lack app/network dispatch; League is intentionally display-only/disabled in the C++ lobby. |
| Password and Comment lobby input dialogs | `C4Network2Dialogs.cpp:718-776` | **Partial** | Exact InputDialog and selector callbacks exist, but the lobby callers and network password/comment mutation are not wired. |
| Right tab icon buttons | `C4GameLobby.cpp:255-264,922-998` | **Partial** | Players/Teams/Resources/Options/Scenario/IRC requests are emitted; only Players content is rendered. |
| Right-caption tab context popup | `MainDlg::OnRightTabContext` (`C4GameLobby.cpp:844-866`) | **Missing** | This C++ popup is flat—Players, optional Teams, Resources, Options—not recursive. The component only emits `TabContextRequested`; Scenario remains a direct tab button. |
| Players/clients roster layout and scrolling | `C4PlayerInfoListBox.cpp:39-488,1237-1607`; `game_lobby.rs` | **Partial** | Component renders the visible client/player hierarchy, state icons, selection, focus and scroll input. Live model updates and all requested child actions need app owners. |
| Player row context root | `C4PlayerInfoListBox.cpp:490-533` | **Missing** | Conditional Take Over, Remove and New Color entries with exact permissions/tooltips. |
| Nested Take Over submenu | `C4PlayerInfoListBox.cpp:535-572` | **Missing** | Recursively enumerate free savegame players and dispatch takeover by player ID. |
| Player Remove control | `C4PlayerInfoListBox.cpp:574-600` | **Missing** | Host/control authorization, countdown lock and direct synchronized removal. C++ does not open a confirmation dialog here. |
| Player New Color action | `C4PlayerInfoListBox.cpp:602-616` | **Missing** | Generate and send the synchronized player-color change. |
| Player team combo/dropdown | `C4PlayerInfoListBox.cpp:618-646` | **Partial** | Component exposes team selection requests and row affordance; classic ComboBox popup, permission/filter rules and synchronized team change are missing. |
| Client rows/status/ping/sound/add button | `C4PlayerInfoListBox.cpp:737-916` | **Partial** | Component draws bounded row variants and emits Add Player/context requests. Ready/status events and APIs now exist at transport/app boundaries, but no lobby model applies them; live ping/sound updates and commands also remain. |
| Client context Mute/Kick/Activate/Info | `C4PlayerInfoListBox.cpp:918-979` | **Missing** | Flat conditional popup, direct lobby kick, activation control, mute state and detail dialog. League voting belongs to the separate runtime client list, not this lobby popup. |
| Add Player selector | `C4PlayerInfoListBox.cpp:981-985`, `MainDlg::OnClientAddPlayer` | **Missing** | Player-file selector, target-client handoff, countdown abort and errors. |
| Team header rows and move-local-players action | `C4PlayerInfoListBox.cpp:989-1110` | **Missing** | Rust has no Team row identity or renderer. Implement team-filtered roster construction, header presentation, double-click bulk move and synchronized dispatch. |
| Free-savegame player group | `C4PlayerInfoListBox.cpp:1112-1143` | **Partial** | Component can present the group; takeover context and restoration are missing. |
| Script-player group and Add action | `C4PlayerInfoListBox.cpp:1146-1210` | **Partial** | Component can present/add-request the row; max-player gating and synchronized script-player creation are missing. |
| Replay-player group | `C4PlayerInfoListBox.cpp:1212-1235` | **Partial** | Component presentation exists; replay-specific state/update ownership is absent. |
| Teams-filtered sidebar mode | `MainDlg::OnTabTeams`, `C4PlayerInfoListBox::PILBM_LobbyTeamSort` | **Missing** | Build the filtered/team-sorted sheet and preserve tab/focus/selection state. |
| Resources list/progress | `C4Network2ResDlg.cpp:32-87,158-216` | **Missing** | Live resources, status icons/progress, activation timer and scrolling. |
| Resource Save/overwrite/success/error branch | `C4Network2ResDlg.cpp:88-157` | **Missing** | Save button, path handling, overwrite confirmation and completion/error dialogs. |
| Preload button, automatic preload and failure log | `C4GameLobby.cpp:231-245,766-842,1001-1016` | **Missing** | Readiness gating, automatic/manual preload and red failure logging. |
| Options list container | `C4GameOptions.cpp:28-310` | **Missing** | List layout, one-second refresh and ComboBox plumbing. |
| Options / Control mode | `C4GameOptionsList::OptionControlMode` | **Missing** | The lobby constructs the non-runtime list, so this row is display-only/read-only there; runtime central/decentral/async selection belongs to other callers. |
| Options / Control rate | `OptionControlRate` | **Missing** | Values 1–9, synchronized adjustment and refresh. |
| Options / Runtime join | `OptionRuntimeJoin` | **Missing** | Barred/free choices, config persistence and live allow-join. |
| Options / Team distribution | `OptionTeamDist` | **Missing** | Scenario-provided choices and synchronized selection. |
| Options / Team colors | `OptionTeamColors` | **Missing** | Enabled/disabled choice and synchronized update. |
| Options / Random team count | `OptionRandomTeamCount` | **Missing** | Conditional values, team recreation and dependent option refresh. |
| Scenario-description sheet | `C4GameLobby.cpp:64-137`, `ScenDesc` | **Missing** | Timed TextWindow updates showing loading percentage, load error, RTF/plain description, or scenario title, plus scrolling. The C++ sheet has no portrait/preview child. |
| Optional IRC/chat window button | `MainDlg::OnBtnChat`, `C4ChatDlg` | **Missing** | Open/raise the full IRC dialog without conflating it with lobby chat. |
| Per-client information dialog | `C4Network2Dialogs.cpp:42-110`, `C4Network2ClientDlg` | **Missing** | Text-only client ID/name/nick/address/status/version/connection information. Host actions belong to `C4Network2ClientListDlg`. |

## In-game main-menu subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player/observer main page | `C4MainMenu::ActivateMain` | **Partial** | Condition data remains hardcoded. Every unsupported child now returns a distinct typed parity error through production input instead of reopening/closing the root, setting status text, or using `NoOp`. |
| Goals list | `ActivateGoals` | **Partial** | Rust omits descriptions, hardcodes fulfillment false and silently loses declared-image decode failures. Selection now fails at a typed GoalInfo boundary instead of warn-closing. Recursively, 101 of 110 shipped Goal/Rule definitions define `Activate`, reaching Info/Dialog/Normal menus, chooser trees and side effects. |
| Rules list | `ActivateRules` | **Partial** | Rule descriptions, Captain fulfilled markers where applicable, live-object selection and the recursive `Activate(player)` subtree remain; selection fails typed meanwhile. Legitimately blank definition pictures reserve an empty symbol in C++; malformed declared graphics must fail closed. |
| Hostility | `ActivateHostility` | **Fail-fast** | Player list and queued hostility toggle are absent; activation returns a typed child boundary. |
| Initial team selection/team switch | `C4Player::ActivateMenuTeamSelection` | **Fail-fast** | `TeamInfo` retains the recursively inventoried `IconSpec` recipes and the root now emits a typed TeamSelection action instead of `NoOp`; activation fails typed until team rows, availability, queued selection and view preview exist. |
| Observer target/free view | `ActivateObserver` | **Fail-fast** | Free/player rows, selection preview and camera transition are absent; activation returns a typed child boundary. |
| Runtime player join | `ActivateNewPlayer` | **Fail-fast** | The misleading always-empty page and status-only Join path are unreachable; both activation and modeled Join actions fail typed until discovery and local/network dispatch exist. |
| Ten save slots | `ActivateSavegame` | **Partial** | Functional page; grouped naming/path/localization semantics differ. |
| Options | `ActivateOptions` | **Partial** | Visible toggles exist; config/localization persistence incomplete. |
| Display | `ActivateDisplay` | **Partial** | Several toggles are state-only and do not affect rendering. |
| Host disconnect/client kick | `ActivateHost` | **Fail-fast** | Client list and removal/vote controls are absent; activation returns a typed child boundary. |
| Client disconnect | `ActivateClient` | **Partial** | The Yes/No page exists; Part now returns a typed child boundary instead of setting status until teardown/vote is implemented. |
| Surrender | `ActivateSurrender` | **Partial** | Offline works; network/league activation returns a typed child boundary instead of setting status until queued control/vote semantics exist. |
| Abort/restart/no | `C4Game.cpp:3428`; `C4GameDialogs.cpp:33-128`, `C4AbortGameDialog` | **Fail-fast** | Bare unmodified runtime Escape and the main-menu Abort action reach the same logged typed boundary instead of opening the wrong C4MainMenu-shaped pane. The menu-shaped approximation is unreachable. Exact dialog halt state, restart policy, league vote subtree and cancel semantics remain unported. Default macOS SDL window-close still exits directly like the platform oracle. |
| Mouse hit testing/scrollbars/tooltips | `C4Menu` | **Partial** | Keyboard navigation works; pointer/scroll behavior remains incomplete. With player mouse control disabled, Rust can still route a left gesture into world mouse-down instead of keeping mouse control inactive. |
| In-game/script menu font and sheet boundary | `C4GUI::Resource`, `C4Menu::Draw`; `lc-frontend/src/ingame_menu.rs`, `lc-app/src/main.rs` | **Partial** | Render and script-menu pointer paths preflight the current font/sheet set; pointer failures are typed and cannot fall through into world input. Required Captain graphics are still omitted, and Normal/Context item images can remain unresolved. |
| HUD resource boundary | `C4GraphicsResource`, player HUD; `lc-frontend/src/hud.rs` | **Missing** | The Rust HUD can synthesize fallback surfaces. Replace that reachability with a logged error until classic resources load. |

## Object-menu subtree

Generic app-owned inventory/get panes are rejected at request creation and at
render time. Live Activate/Get requests cannot retain synthetic state or run
its actions before a frame is rendered.

| Menu identification/style | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Construction (`C4MN_Construction`) | `C4Command::Construct`; `C4Object::ActivateMenu` | **Partial** | Definition-less Construct now follows the shipped context action, applies the CanConstruct gate, closes the prior object menu, completes successfully and reaches a typed/logged app boundary. The engine-owned knowledge/component menu, live scheduled vehicle rows, selection and Construct-definition children remain missing; no generic Rust pane is rendered. |
| Activate (`C4MN_Activate`) | `C4Command::Activate`; `C4Object::ActivateMenu`; `C4ObjectMenu` | **Fail-fast** | Implicit Take resolves its current container and shipped Kayak Target2 retains its explicit container; both route by Controller, force-close the prior menu and honor `RejectContents`, then return a typed boundary before app-owned inventory state exists. Grouped activation rows, primary/secondary commands, refill and close remain unported. |
| Get (`C4MN_Get`) | `C4ObjectMenu` | **Fail-fast** | Local requests return a typed boundary before app-owned container state exists; the classic menu remains unported. |
| Buy (`C4MN_Buy`) | `C4ObjectMenu` | **Partial** | Strong classic page; exact ordering/dynamic value/availability. |
| Sell (`C4MN_Sell`) | `C4ObjectMenu` | **Partial** | Exact eligibility/value/refill behavior. |
| Context (`C4MN_Context`) | `C4ObjectMenu` | **Partial** | Pushed/remote/construction/action/effect/attachment/crew nested cases. |
| Info (`C4MN_Info`) | `C4ObjectMenu` | **Partial** | Classic target picture/title, one-shot target-relative placement, rich wrapped `InfoCaption`, and ordered static + `Fx*Info` text render in production. Shipped paths are covered; remaining mod-facing gaps are effect-list mutation during `Fx*Info`, italic/image markup transforms, and arbitrary text-image grammar. |
| Contents (`C4MN_Contents`) | `C4ObjectMenu` | **Partial** | CollectionLimit/RejectCollection switching and exact refill. |
| Selection-follow scrolling/pointer drag | `C4Menu`, `C4ObjectMenu` | **Missing** | Scrollbars, wheel/thumb, construct drag. |

## Script-created menu grammar

| Feature | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Normal style | `CreateMenu`, `AddMenuItem` | **Partial** | Classic grid geometry, selection, shipped image recipes, decoration, progressive text and title/tooltip TextSpec images render in production. Remaining: generic pointer/scrollbars, callback gaps, and the portrait-state forms listed below. |
| Context style | same | **Partial** | Classic layout, command strip, pointer targeting, shipped image recipes and caption/title TextSpec images render in production. Remaining: exact scrolling, nested callback cases, and late-layout ObjectRank height for mod-created menus. |
| Info style | `C4MN_Style_Info` | **Partial** | Classic width/row geometry, pictures, markup-aware wrapping, complete TextSpec image grammar, pointer targeting, no highlight/tooltip, close-only footer, and fail-fast image preflight render in production. All shipped paths create exactly one row; 137 `MessageWindow` callers across 103 files reach this style. Remaining: exact italic markup transforms, active-scenario resource overrides, and generic multi-row scrollbars. |
| Dialog style | `C4MN_Style_Dialog` | **Partial** | Four direct shipped creation sites fan out through Hazard, LastWill, and Western dialogue helpers. Production uses the classic variable-height layout, portrait column, empty-title rule, pointer targets, selection/progress behavior, decoration path, complete inline TextSpec grammar and shipped image recipes; unresolved requested images or drawable decoration sheets fail fast. A natural Goldrush Talker sequence and real LastWill resources render without fallback. Remaining: natural LastWill/Hazard scenario fixtures and current-vs-permanent portrait-state gaps; all shipped Dialog sites use `extra=0`, so the optional footer is mod-facing. |
| EqualItemHeight flag | `C4MN_Style_EqualItemHeight` | **Complete** | The raw style bit is preserved independently of base style and Dialog symbol rows use the C++ equalization/restacking rule. |
| Rank/Indexed/ObjRank/Object/TextSpec/Color symbols | `AddMenuItem` image grammar | **Partial** | Shipped calls use exact fallback-definition resolution, clipped indexed phases/colors, inherited rank strips and extended/captain ranks, and add-time Object/ObjectRank snapshots. Snapshots include temporary objects, foreign graphics/overlays/transforms, `ChangeDef`, color modulation, and Dialog's 64-pixel symbol size. All 75 shipped definition portraits are recursively inventoried/loaded in group order, and natural Western/LastWill paths are covered. TextSpec now shares one exact parser for uppercase/underscore IDs, scanf-style indexed phases, named portraits with real bitmap validation, and all seven prefix-matched `Ico:*` forms; inline images participate in title/body measurement, rendering, tooltips and pointer layout for every style, including Goldrush's shipped `SBTR` Context rows. Remaining shipped gap: persisted/current-vs-permanent crew portraits used by Knights/Hazard. Remaining mod-facing gaps include late-layout Context ObjectRank height, active-scenario graphics/font overrides, and owned-pixel snapshots if asset hot reload is introduced. |
| TextSpec grammar/corpus | `C4Game::DrawTextSpecImage`, `C4DefList::GetFontImage` | **Complete** | Parser tables cover C scanf prefixes, raw portrait IDs/colors, case rules and icon prefixes; recursive shipped team census covers 39 `IconSpec` values in 19 files. Keep the Goldrush Context render/hit-test regression current. |
| Footer extras (`C4MN_Extra_*`) | `C4Menu::DrawElement` | **Partial** | Value, magic, components and close/command strips are modeled; retain explicit coverage for every extra kind and dynamic refill source. |
| Explicit size/permanence/alignment | `SetMenuSize`, `SetMenuPermanent`, `CreateMenu` alignment | **Partial** | Core fields survive; finish free-position Context requests, movement/clamping and close semantics. |
| Primary/secondary commands | `C4MenuItem::Command`, `Command2` | **Partial** | Engine preserves both command strings; complete right-click dispatch through every menu style and nested callback path. |
| Menu decoration | `SetMenuDecoration` | **Partial** | `SetByDef` snapshots callback-derived color/borders and all eight `FrameDeco*` facets immediately; every classic style applies packed alpha, tiled/truncated edges, protruding corners, clipped out-of-bounds source facets, and captured margins. Background-only decoration is valid; unsafe geometry and unresolved drawable facets fail fast. Real LastWill assets and the natural Western Goldrush dialogue path are covered. Remaining: definition hot-reload refresh/clear behavior when Rust gains that API. |
| Progressive text | `SetMenuTextProgress` | **Complete** | Per-row byte offsets, portrait exclusion, markup-skipping shared budgets, selectable-row reveal, late-added rows, one-byte menu ticks, explicit show-text, local-only first-input conversion, and byte-prefix Dialog rendering are modeled and tested. |
| Selection/close/command callbacks | `C4Menu`, script callbacks | **Partial** | Object-owned `MenuQueryCancel` and `OnMenuSelection` work on the covered host paths. Finish engine-side close/selection/command dispatch and the remaining no-command-object `CB_Scenario` paths; do not describe all `MenuQueryCancel` handling as absent. |
| Script-menu resource/font fallback boundary | `C4Menu::Draw`, `C4GUI::Resource`; `ingame_menu.rs` | **Partial** | Render preflight blocks several fallback paths. Pointer targeting now shares the exact global-resource preflight and returns a typed, logged boundary for unresolved inline images before hover, left, or right input can reach the world. Normal/Context unresolved item pictures remain accepted. |

## Other game-visible dialogs and overlays

| Screen | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Game over/evaluation | `C4GameOverDlg` | **Partial** | The game-over Evaluation dialog is running-mode only (the result computation itself can still occur during game clear): a forged stale `game_over_dialog` in Menu now returns a typed boundary carrying the exact startup root before startup bootstrap/view/model/child/status, matching-cache replay, or logical/native pixels. All seven `StartupView` roots—including retained Network Chat, Options Graphics and About Licenses children—are exhaustive regressions; invalid lifecycle also wins before game-over resource validation and the `GameOverResources` boundary, matching C++'s exclusive startup/running lifecycles. The eight unconditional running-mode core/dialog/icon resources (`CStdFont/Endeavour.ttf`, `GUICaption.png`, `GUIButton.png`, `GUIButtonDown.png`, processed `GUIButtonHighlight.png`, `GUIIcons.png`, `Player.png` and the HUD `Score.png` source) retain their ordered fail-fast preflight before viewport collection or pixel output. Later unported branches still need a recursive resource inventory for `GUIIcons2.png`, `Crew.png`, `GUIScroll.png` and dynamic definition/player images. Global/per-player custom evaluation, two-team lists, team/league rows, live network result/streaming updates and exact host/film button policy also remain. |
| Game-over chat input | `C4GameOverDlg.cpp:317-321`; `C4MessageInput` | **Fail-fast** | Exact bare Return/F2 and Shift+Return reach typed, logged All/Allies boundaries before any evaluation button can activate. When signed `[Controls] GamepadGuiControl` is nonzero, raw gamepad-0 `AnyLow` Press likewise reaches All chat, so East remains chat even though its later abstract alias is Cancel; disabled GUI control and every literal pad ID other than zero are inert throughout the exclusive evaluation GUI stack. Logo is outside the C++ modifier mask and keypad Enter is not silently aliased on the macOS SDL route. Exact Alt/Alt+Shift probes first reach a separate typed mnemonic boundary because SDL derives a letter from the key name and visible localized buttons outrank the global Say binding (for example Return can activate `&Restart`). The recursive edit screen, history, completion, paste/clipboard context popup, submit/cancel lifecycle and network transport remain unported. The shared command-language descendants are inventoried below; notably `/chart` opens the tabbed `C4ChartDialog` and league `/kick` queues a `C4VoteDialog` with Yes/No dispatch. |
| Game-over focus and keyboard/gamepad lifecycle | `C4GUI::Dialog`, `C4GameOverDlg` | **Missing** | The representable initial no-focus state keeps keyboard arrows and Space inert even after pointer hover, releases are consumed, and exact Tab/Shift+Tab reach typed forward/backward traversal boundaries. Classic and fallback evaluation buttons now start unhighlighted: pointer hover is separate, clears outside/on leave, and does not become keyboard focus; same-target pointer press/release activation remains. Classic down pixels require the retained press latch and current hover, so drag-out raises the button and re-entry depresses it before release. ArrowHit/Click GUI sound dispatch is still unported. With nonzero signed `GamepadGuiControl`, raw gamepad-0 Left/Right Press reach exact backward/forward typed traversal boundaries, Up/Down and releases/Clear are inert, `AnyHigh` Press invokes End, and direct abstract Action/Command aliases never activate a button. Input precedence remains message, definition selector, context menu, input dialog, evaluation, then base screen; root-context Left pass-through reaches evaluation traversal. Production input preserves the physical gamepad ID and event-cluster ID: one top owner remains sticky for a GUI button and its Action/Command/Clear aliases even if it closes, while every Direction and the next GUI/standalone event receive a fresh cluster and re-resolve against the exposed screen. This distinguishes a RightTrigger2 High+Clear cluster from a standalone disconnect Clear, handles an axis release/opposite-press pair as two dispatches, and never persists alias ownership across polls. Exact Alt/Alt+Shift keys reach the higher-priority mnemonic boundary; other Ctrl-modified Tab and modified Escape combinations remain inert, while bare Escape retains End. True none/list/button focus ownership, traversal, localized mnemonics, button keyboard press/release visuals and activation remain missing. |
| In-game chat/script query input | `C4MessageInput` | **Missing** | Modes, history, completion, paste, team/private/action/sound/alert/say and script query remain. Recursively inventory the edit context menu and the full slash/custom dispatch: `/msgboard` changes message-board screens, `/clear` mutates log surfaces, `/observer` changes viewport/player state, `/fast` and `/slow` reach timing/feedback, `/set password\|comment\|faircrew\|maxplayer` changes lobby/reference state, `/chart` opens `C4ChartDialog`, league `/kick` can queue `C4VoteDialog`, and script/custom commands call scenario handlers. Network chat also awaits the safe `CID_Message` codec and roster validation. |
| Scoreboard dialog | `C4Scoreboard`, `C4ScoreboardDlg` | **Fail-fast** | The saved row-major matrix, header keys, signed show refcount and Set/Get/Sort/ScoreboardCol APIs are modeled. `DoScoreboardShow` now returns without even changing the refcount during invalid/exclusive startup and save loading, then emits ordered call-time reconciliation records only after the final shared-mode snapshot; SetCell alone never opens a dialog, open-then-close in one tick leaves none, game over force-closes without decrementing, Continue does not reopen, and a later runtime reconciliation may. Bare or Logo+Tab honors dialog/context/player-control priority, exact modifiers, zero-vs-negative eligibility and can close an already-open dialog; an absent eligible dialog or any script-visible surviving dialog reaches a typed, logged boundary without cell text or pixel/UI fallback. Remaining recursive renderer/lifecycle: custom `C4KeyConfig` ScoreboardToggle remapping; optional `(0,0)` wooden title and Player icon, including the allocated-empty-title stale-margin quirk; live FontRegular measurement/invalidation and markup/TextSpec definition-image dependencies; 4-pixel cell indent, 3-pixel outer margins, left first-column and centered later-column text; row/column insertion, pruning and reflow while open; title close/drag/tooltips/sounds; immediate `Show`/`Close(false)` delete-on-close and `pDlg` destructor ownership; non-mouse-controlled behavior; the absence of table scrolling/clipping; console-mode native-child handling; render z-order; and top-right placement against the live `Screen::PreferredDlgRect` (which may be the mouse-controlled player viewport), realigned on show/data Update rather than merely when that preferred rect changes. |
| Upper board modes and right-side indicators | `C4UpperBoard::Init`, `Draw`, `Height`; `lc-frontend/src/hud.rs` | **Partial** | Rust renders a substantial Full-mode board, scenario title and game time. Implement Hide/Small/Mini geometry and texture/logo scaling, the Mini message-board split, wall clock and FPS; the in-game Display toggle currently changes state without changing this renderer. |
| Player and cursor HUD subtree | `C4Viewport::DrawCursorInfo`, `DrawPlayerInfo`, `DrawPlayerControls`, `DrawPlayerStartup`; `lc-frontend/src/hud.rs` | **Partial** | Portrait/rank/name, inventory, energy/magic/breath, command rows, wealth/score/crew, tutorial control hints and startup name have classic-shaped renderers. Finish `ShowPortraits`/`ShowPlayerHUDAlways`, definition `HideHUDElements`/`HideHUDBars`, mouse/player-menu command regions, every dynamic visibility rule and exact resource ownership. This behavior status is separate from the Missing fallback boundary in the in-game menu section. |
| Eliminated/surrendered viewport message | `C4Viewport::DrawMenu:970-975` | **Missing** | Rust retains eliminated viewport records but does not draw the centered red localized eliminated/surrendered text or suppress the ordinary menu path as C++ does. |
| Object-independent `NO_OWNER` observer viewport | `C4FullScreen::ViewportCheck`; `C4Viewport` | **Fail-fast** | Viewport collection now preserves every declared local player and slot in order (including eliminated players, exact center and zoom) and resolves only the slot focus, player cursor, then first crew object. Before gamma/overlay/frame rendering or pixel output, one typed boundary recursively distinguishes a zero-object world; a declared local owner with no player or no viewport; an exact owner/slot with no live focus, cursor or crew object (including mixed valid/invalid locals and slots); and the unavailable object-independent observer camera with the full declared-local ID payload. None may silently drop a local/slot, invent a first-object focus or selection mark, synthesize an `OWNER_NONE` object focus, or reach the solid navy app fallback. The exact object-independent camera remains unported. |
| F1 two-column game-help overlay | `C4Game.cpp:3378`; `C4GraphicsSystem::DrawHelp` | **Fail-fast** | Bare or Logo+F1 in Running mode now logs and returns a typed `RuntimeHelpToggle` boundary before message/context/evaluation/object/player-menu input or any UI mutation; releases are consumed and repeated presses may reach the boundary again. Alt/Ctrl/Shift variants retain downstream priority, and Menu/Loading are excluded. The exact 17 displayed rows are recursively inventoried: Help; music; sound; F4 network client list; the two live `ChatOpen` all-chat labels; message-history back; message-history forward; IRC; full scoreboard; normal and extended screenshots; speed-up and speed-down feedback; debug mode; combined entrances/vertices; combined actions/commands/pathfinder; and solid masks. Every label is resolved live through `GetKeyboardInputName`; the oracle asks for `GameSpeedDown` although registration uses `GameSlowDown`, so its speed-down label is blank. Rust `KeyboardBindings`/`KeyConfig` cannot yet represent custom F1 player/global remaps, and the exact overlay geometry, draw order, localization and remapping priority remain unported; no help-visible boolean or generic renderer is invented. |
| Pause/hold overlay | `C4Game::TogglePause`, `Pause`, `Unpause`; `C4GraphicsSystem::DrawHoldMessages` | **Fail-fast** | Bare or Logo+Pause is now handled as a runtime-global key before every modal, context, object, player or evaluation dialog. It is the exact no-op while game over is shown. Otherwise it logs and returns a typed route: offline cannot mutate the unavailable `Game.HaltCount`; a consistently identified host/client cannot choose safely because runtime league/evaluated state is unavailable; inconsistent network manager/mode/client-ID combinations use a distinct unknown-role boundary. This deliberately never claims non-league mode: league Pause/Unpause recursively starts a `VT_Pause` vote, while non-league host changes network status, client returns without mutation and offline toggles the halt count. Releases are consumed and repeated presses can reach the boundary again. The centered Pause overlay, halt/status ownership, console toolbar synchronization and vote subtree remain unported. |
| Timed flash-message overlay | `C4GraphicsSystem::FlashMessage`, `DrawFlashMessage` | **Missing** | Port timing, placement below the upper board/multiple viewports and background invalidation. Observer/gamepad prompts, runtime-join state, speed changes and debug toggles recursively reach this screen. Generic startup/running status text is not a substitute. |
| In-game mouse-control/cursor overlay | `C4MouseControl::Draw`; `C4Viewport::DrawMouseButtons` | **Missing** | Port classic cursor phases, target/help captions and tooltips, selection and construction-drag visuals, clipping/FoW behavior, and Help/Player Menu/Chat command regions. Loading the cursor atlas for player selection flashes does not implement this mouse screen. |
| Debug and network-status viewport overlays | `C4Viewport::Draw`; `C4GraphicsSystem::ToggleShow*`; `C4Network2::DrawStatus` | **Missing** | Ctrl+F6/F7/F8 and `NetStatsToggle` reach entrances/vertices, actions, commands, pathfinder, solid masks and detailed network state. Track their debug/console permission gates, flash feedback and render ordering. |
| Rust-only FRAME/POS/VEL debug HUD | no C++ visual authority; `LC_APP_HUD_DEBUG`, `GraphicsSystem::draw_hud` | **Missing** | This explicit environment route exposes Rust status strings from object-menu actions, unsupported in-game main-menu actions, save/load, network/desync and right-click failures, game-over/load transitions and focus diagnostics. Keep it developer-only and documented, or reject it in parity runs; it is not parity evidence and never substitutes for help, flash, network status or other classic overlays. |
| Loader/progress/log screen | `C4LoaderScreen.cpp:126-177`; `lc-frontend/src/loader_screen.rs`; `lc-app/src/main.rs` | **Partial** | The resource-validating frontend and app wire classic group priority/order, weighted loader selection, extension-directed decode, title and GUI-resource precedence, user gamma, resource refresh, logical chrome and scale-native text through the two-pass renderer. Unsupported Extra/language-pack ambiguity fails closed. The Rust workers still do not own C++'s live boot/scenario progress, process text or log-buffer milestones, and non-integer `Config.Graphics.Scale / 100.0f` remains a typed boundary even though C++ accepts it. |
| Loader generic-fallback boundary | `C4LoaderScreen`; `lc-app/src/main.rs::render_loading` | **Fail-fast** | Every missing selection, resource, font, unsupported scale and render failure is logged and returned as a typed error before the old generic pane can draw. This guard remains required for the unresolved Partial states above; it is not evidence that live progress/log behavior is complete. |
| Object/global game messages (`C4GameMessage`) | `C4GameMessage.cpp:38-231` | **Fail-fast** | Every drawable local/observer viewport is checked, including duplicate-owner split views and C++ parallax projection. Drawable messages return a typed, logged boundary; target visibility/FoW snapshots that cannot be classified exactly use a separate fail-closed boundary. Remote-only, missing-target and logically offscreen messages do not overblock. The exact renderer remains unported. |
| Message board modes/history | `C4MessageBoard` | **Partial** | Only one current line; multiline/history/scroll missing. |
| Runtime per-client information dialog | `C4Network2ClientDlg` | **Missing** | Text-only client identity, status, address/version and connection information. |
| Runtime client list | `C4Network2ClientListDlg` | **Fail-fast** | Bare or Logo+F4 in Running mode logs and returns a typed `RuntimeClientListToggle` boundary before game-over, message, context, object and player-menu handlers; the payload conservatively distinguishes Offline, consistent Host (manager ID 0), consistent Client (nonzero ID), and every ambiguous combination. Releases are consumed, repeats may reach the boundary again, and Alt/Ctrl/Shift-modified keys retain downstream priority. Remaining recursive screen: the game-options list; client rows and host/client/observer/loading/ready/kick/wait status icons; name/nick/player tooltips and ping; mute, activate/deactivate and host kick actions (including league vote gating); data/message connection rows with transport, peer, loss and ping; league-gated disconnect; live Tick/Behind/Rate/PreSend/ACT footer; focus, selection, timer updates, Escape and singleton toggle/close lifecycle. This remains distinct from the per-client information dialog. |
| Runtime ready-check timed dialog/toast | `C4Network2::ReadyCheckDialog`, `C4Network2.cpp:129-198,1625-1690` | **Missing** | `PID_ReadyCheck` transport and ready APIs are Partial above, but this screen is absent: participant state text, timeout, toast action and app response binding remain. |
| Network vote dialog | `C4VoteDialog`, `C4Network2::Vote`, `AddVote`, `EndVote`, `OpenVoteDialog` | **Missing** | Recursively port original-vote rate limiting, duplicate-vote suppression, direct `CID_Vote` dispatch, host pause-for-vote ownership, joined-local-player eligibility, ordered pending votes, localized `VT_Cancel`/`VT_Kick`/`VT_Pause` text, Yes/No dispatch, matching-dialog close and next-vote replacement, result updates and restart when the final vote releases `fPausedForVote`. Rejected own kick/cancel votes can branch into the league surrender confirmation. Pause and unpause each use this subtree in an unevaluated league game. |
| League surrender confirmation | `C4Network2::OpenSurrenderDialog`, `LeagueSurrender` | **Missing** | Forfeit-specific confirmation and disconnect/report behavior. |
| Network statistics chart | `C4ChartDialog`; `C4Network2Stats` backing model | **Missing** | OC/FPS/IO/ping/control/APM tabs. |
| League signup/register/auth form | `C4LeagueSignupDialog`, `C4League.cpp:548-749` | **Missing** | Account/password/confirm/check-password fields, validation, warning and modal return. |
| League waits/retries/results | `C4League`, network dialogs | **Missing** | Request progress, retries, authentication failures, score/rank/result screens and disconnect reporting. |
| Reusable URL/download progress dialog | `C4DownloadDlg` | **Missing** | Port transfer progress, cancel, completion/error ownership and the updater callers. Lobby resource status/save remains the separate `C4Network2ResDlg` subtree above. |
| File/player/definition/portrait selectors | `C4FileSelDlg` family | **Partial** | The multi-select definition specialization is complete, including its recursive error modal and runtime handoff. Single-file, player and portrait specializations, locations/combo boxes and portrait options remain. |
| Rust-only F5 quick save | no C++ menu equivalent; C++ uses `C4MainMenu::ActivateSavegame` | **Fail-fast** | The shortcut logs and returns a typed boundary instead of bypassing the classic ten-slot flow. |
| Rust-only F9 quick load | conflicts with C++ `C4Game.cpp:3373-3374` | **Fail-fast** | The shortcut no longer quick-loads; it logs and returns a typed boundary. Restore the C++ screenshot action and keep loading in an oracle-backed menu. |
| C++ F9 viewport screenshot | `C4Game.cpp:3373`; `C4GraphicsSystem::SaveScreenshot(false)` | **Missing** | Capture the current back surface with classic scale/gamma semantics, choose the first free `ScreenshotNNN.png`, and emit localized success/error feedback. |
| C++ Ctrl+F9 whole-landscape screenshot | `C4Game.cpp:3374`; `C4GraphicsSystem::SaveScreenshot(true)` | **Missing** | Port tiled full-landscape capture, temporary observer viewport/parallax changes, page flips, restoration, PNG errors and feedback. It is a distinct action from ordinary F9. |
| Rust-only F6/F7 named save/load browser | no C++ equivalent; `lc-frontend/src/save_browser.rs` | **Fail-fast** | The renderer remains in-tree, but the app guard rejects both shortcuts. Remove it or replace it with the classic ten-slot flow; never relax the guard. |
| Rust synthetic menu sandbox | no C++ player-facing equivalent; `lc-app/src/main.rs` | **Complete** | Empty discovery/catalog paths no longer inject the sandbox into player-facing menus. The explicit `--sandbox` developer/test route remains intentionally separate from menu discovery. |

## Required generic-fallback boundaries

Until the classic screens above are wired, central app boundaries should group
the remaining unsafe presentation routes rather than adding more status-only
call-site checks:

- startup status substitute: include the active startup view and producer/message;
- running viewport unavailable: distinguish zero objects, each missing/empty
  declared local viewport, each owner/slot with no live focus and the
  unavailable object-independent observer camera without inventing a focus
  object;
- game overlay unavailable: identify Help, Pause, Flash, Mouse Control,
  Eliminated, Upper Board mode and debug/network status;
- HUD resources unavailable: report the complete missing classic resource set
  before any generic font, solid surface or omitted image can draw.

The boundary is reached only when that presentation would be visible. Merely
retaining an old renderer in the tree is acceptable when a tested guard makes
it unreachable.

## Reusable modal infrastructure required

Full recursive parity requires classic implementations of these C++ dialog
families before individual call sites can be considered complete:

The ordinary classic message-dialog foundation is now **Partial**: regular,
medium and small geometry; normal/extended icons; canonical OK/Retry/Cancel/
Yes/No ordering; title-close; active-only stacked focus; keyboard, Alt hotkeys,
mouse, touch and legacy low/high gamepad routing; GUI sounds; no-scrim rendering;
continued underlying timers; fail-fast asset validation; and the localized-label
capable “don't show again” checkbox are implemented. The natural network empty-
selection error and player-delete Yes/No/error chain use it. Active scenario
`Graphics.c4g`/font overrides, localized standard button strings, caption drag/
autoscroll and the other call sites remain.

- message/error/info (**Partial**: reusable message/error base, two natural paths);
- Yes/No and multi-button confirmation (**Partial**: all button shapes exist;
  player deletion has a typed continuation);
- “don't show again” checkbox (**Partial**: exact component exists; its five
  shipped config-bound callers remain);
- text/password input (**Partial**: `lc-frontend/src/input_dialog.rs` provides
  the resource-validating classic controller/renderer and network-selector
  Password/Comment are app-wired; the other C++ callers and active-scenario
  resource overrides remain);
- timed confirmation (**Missing**);
- wait/progress with cancel (**Missing**);
- scrolling text/info (**Missing**);
- context and nested context menus (**Partial reusable chassis**: exact classic
  geometry/assets, recursive submenus, deepest-first pointer routing,
  keyboard/hotkeys, touch and low/high gamepad input, flip placement,
  selection/tooltips/sounds, focus suppression, outside-down pass-through,
  action-release capture and fail-fast resources are implemented; startup
  player-row Properties/Delete, the recursively lazy startup Participants
  Add/Remove tree, and the scenario-search Edit actions are callers. Remaining
  consumers include combo boxes, crew rows, lobby right tabs, and runtime
  player/client takeover trees);
- combo-box dropdowns (**Missing**: `C4GUI::ComboBox::DoDropdown` is a context
  menu consumer and must use the same chassis);
- file/image/player/definition selection (**Partial**: definition selection is
  complete; the other `C4FileSelDlg` specializations remain).

Unsupported entry points must keep logging and returning an error until their
classic implementation lands. Reintroducing a generic pane is not completion.

## Editor-only GUI, tracked separately

The Rust startup editor/controller substitute is not this GUI and does not make
any row Partial. Until these branches exist, editor entry points must be
explicitly rejected instead of opening a synthetic controller or an external
editor as if parity had been reached.

| Screen or recursive branch | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Console shell, native menu bar, log, script entry and status bars | `C4Console.cpp:277-578` | **Missing** | Window lifetime/position, log scrolling, command/script dispatch, cursor label, frame/script/time/FPS status and enable rules. |
| Play/Halt and Play/Edit/Draw mode toolbar | `C4Console::InitGUI`, `UpdateHaltCtrls`, mode callbacks | **Missing** | Pause ownership, mutually exclusive tool modes, icons and editing/network gating. |
| File / Open | `C4Console::FileOpen` | **Missing** | Native scenario selector, close/current-game confirmation, load errors and recent state. |
| File / Open with Players | `C4Console::FileOpenWPlrs` | **Missing** | Scenario selection plus recursive player-file selection and start transition. |
| File / Save Scenario and Save Scenario As | `C4Console::FileSave`, `FileSaveAs(false)` | **Missing** | Component packing, path selector, overwrite/error dialogs and current path update. |
| File / Save Game and Save Game As | `C4Console::FileSave`, `FileSaveAs(true)` | **Missing** | Runtime/player eligibility, savegame path/overwrite and error branches. |
| File / Record | `C4Console::FileRecord` | **Missing** | Runtime-record eligibility, filename flow and record-state transition. |
| File / Close and Quit | `C4Console::FileClose`, `FileQuit` | **Missing** | Unsaved/running confirmations, teardown and application quit. |
| Components / Objects | `C4Console::EditObjects`, `C4ObjectListDlg` | **Missing** | Open/update the recursive Object List screen described below. |
| Components / Script | `C4Console::EditScript` | **Missing** | Scenario script component editor, save/reload and syntax/error handling. |
| Components / Title | `C4Console::EditTitle` | **Missing** | Title component text editor and persistence. |
| Components / Info | `C4Console::EditInfo` | **Missing** | Info component text editor and persistence. |
| Player / Join | `C4Console::PlayerJoin` | **Missing** | Player selector, join validation/control and errors. |
| Player / dynamic Quit entries | `C4Console.cpp:1410-1461` | **Missing** | Rebuild per active player, permission-sensitive enable state and eliminate control. |
| Viewport / New | `C4Console::ViewportNew` | **Missing** | Detached viewport construction, placement and close ownership. |
| Viewport / dynamic per-player entries | `C4Console.cpp:1203-1266` | **Missing** | Rebuild by player and create player-targeted viewport. |
| Network / dynamic client entries | `C4Console.cpp:1540-1608` | **Missing** | Insert/remove menu, local/remote labels and host-authorized client removal. |
| Help / About | `C4Console::HelpAbout` | **Missing** | Native developer About dialog. |
| Landscape mode selector | `C4ToolsDlg.cpp:262-407,865-896` | **Missing** | Dynamic/Static/Exact modes and enable/update rules. |
| Drawing tool selector | `C4ToolsDlg` | **Missing** | Brush/Line/Rectangle/Fill/Picker, temporary alternate tool and cursor integration. |
| Drawing material/texture/grade/IFT controls | `C4ToolsDlg.cpp:410-1015` | **Missing** | Material and texture combos, preview, grade slider, IFT toggle, defaults and landscape validation. |
| Object Properties display/script input | `C4PropertyDlg.cpp:103-166,398-520` | **Missing** | Selection-dependent property text, definition reload and object-scoped script command entry. |
| Object List tree/selection | `C4ObjectListDlg.cpp:486-805` | **Missing** | Live add/remove/rename updates, definition icons, multi-selection and edit-cursor synchronization. |
| Shared developer notebook/window | `C4DevmodeDlg.cpp:44-130` | **Missing** | Add/remove/switch pages, title, hide/show, transient ownership and remembered position. |
| Edit-cursor context root | `C4EditCursor.cpp:79-104,582-627` | **Missing** | Permission-sensitive native popup and mode-dependent Properties/Tools label. |
| Edit cursor / Delete | `C4EditCursor::Delete` | **Missing** | Selected-object control dispatch and state refresh. |
| Edit cursor / Duplicate | `C4EditCursor::Duplicate` | **Missing** | Duplicate selection/control semantics. |
| Edit cursor / Grab Contents | `C4EditCursor::GrabContents` | **Missing** | Replace selection with contents, update properties and exit objects. |
| Edit cursor / Properties or Tools | `C4EditCursor::OpenPropTools` | **Missing** | Route by Edit versus Draw mode to the correct developer page. |
| Detached/new viewport windows and viewport input | `C4Viewport`, `C4Console::ViewportNew` | **Missing** | Native windows, player targeting, cursor context and teardown. |
| Platform open/save dialogs | console and component callbacks | **Missing** | Native path selection, filters, overwrite and cancellation semantics. |
| Platform message/error/crash dialogs | console/startup/application error paths | **Missing** | Parent/modal ownership, expandable crash log, restart/fatal variants and localization. |

These may be scheduled after player-facing menus, but literal full C++ GUI
parity is not achieved while they remain absent.

## Verification gates

Every row promoted to **Complete** needs:

1. a C++-referenced behavior test for each transition and nested action;
2. input coverage for keyboard, mouse, wheel/drag, touch where C++ supports it;
3. state restoration/focus/selection/scroll boundary coverage;
4. render evidence using real classic resources at audited resolutions/scales;
5. no reachable generic fallback;
6. recursive scenario/script corpus coverage where the screen is data-driven;
7. table-driven failure coverage proving every startup status producer returns
   a typed error before cache replay, base composition or scale-native text can
   write startup pixels;
8. zero-object, no-local-player and ordinary local-player viewport tests proving
   that the app neither draws the navy empty-state fallback nor invents an
   observer selection target;
9. direct coverage for F1 Help, Pause, Flash, mouse control, eliminated text,
   all four upper-board modes and debug/network overlays, whether implemented
   exactly or guarded by the correct typed boundary;
10. explicit documentation and tests for intentional product divergences such
    as recursive scenario search; they are never silently labeled C++ parity.
