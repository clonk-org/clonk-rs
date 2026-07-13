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
| Main screen font/resource failure boundary | `C4StartupMainDlg`; startup graphics resources | **Fail-fast** | The app logs the complete missing-resource set and returns an error before the component's bitmap-font, optional-image, blue-pane or solid-button substitutes can become player-visible. |
| Dialog switch/fade/back stack | `C4Startup::SwitchDialog`, `C4StartupDlg::OnClosed` | **Partial** | Match fade timing, previous-dialog ownership and every Escape/back transition; Rust currently switches a flat `StartupView`. |
| Common startup cursor/focus/hotkeys/tooltips | `C4GUI::Screen`, `Dialog`, `Control` | **Partial** | Centralize the classic cursor, 500 ms tooltip delay, Tab/Shift-Tab traversal and Alt hotkeys instead of per-screen approximations. |
| Participants Add/Remove context and player submenu | `C4StartupMainDlg::OnPlayerSelContext*` | **Complete** | Exact autosized markup label target and tooltip, recursive lazy Add/Remove tree (including empty 40x7 children), raw filesystem/config ordering, raw Remove indices, player icons/tooltips, activation/validation/deduplication, focus suppression and fail-fast resources are covered. |
| First-run new-player properties modal | `C4StartupMainDlg::OnShown` | **Missing** | Depends on player-properties dialog. |
| Automatic/manual/incoming update flow | `C4StartupMainDlg::OnShown`, `C4UpdateDlg` | **Missing** | See update subtree. |
| F6 editor launch from startup | `C4StartupMainDlg::SwitchToEditor` | **Missing** | Bind and validate the legacy editor launch. |
| Rust editor scenario/controller route | no C++ startup-list equivalent | **Fail-fast** | `ScenarioKind::Editor`, touch activation and `StartupMenuAction::EditEntry` return typed, logged boundaries; the external-editor substitute was removed. Keep the guard until the real developer UI exists. |
| Startup fatal/restart log info dialog | `C4Startup::DoStartup` | **Missing** | Classic expandable info dialog. |

## Scenario selection subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Local scenario book chrome/list/info/buttons | `C4StartupScenSelDlg`; `startup_scensel.rs::ScenselDialogFocus` | **Partial** | Exact focus chrome/order, keyboard/gamepad traversal, disabled-control skipping and pointer/touch hit bounds are covered. Name hotkeys, refresh, rename and delete remain. |
| Recursive `.c4s`/`.c4f`/directory discovery | `C4ScenarioListLoader` | **Complete** | Retain recursive discovery/order tests. |
| Recursive search results and deep-folder Back reconstruction | `C4StartupScenSelDlg::UpdateList`, `Folder::Start`, `FolderBack`; `lc-app/src/main.rs::collect_frontend_search_matches` | **Complete** | Search walks every descendant, and opening a deep result reconstructs every intermediate folder layer so Back pops one level at a time. Keep deep-nesting regressions. |
| Folder navigation and folder metadata | `Folder::Start`, `FolderBack` | **Partial** | Runtime reload/mutation parity. |
| Folder-map view (`FolderMap.txt`) | `C4StartupScenSelDlg.cpp:47-404,1016-1023`, `C4MapFolderData`; shipped `content/Western.c4f/FolderMap.txt` | **Fail-fast** | Direct opens, recursive search/start, packed logical groups and duplicate merged roots inspect every contiguous `.c4f` ancestor/contributor case-insensitively and return a typed, logged boundary before the ordinary book can draw. The actual map background, buttons, overlays, access graphics and info pane remain unported. |
| Search edit/filter | `OnSearchBarEnter`, `UpdateList`, `KeySearch`, `C4GUI::Edit` | **Partial** | Submit-only markup-stripped filtering, Ctrl+F select-all, caret/selection, word edits, clipboard shortcuts, mouse capture/double-click, horizontal scroll, blink, and render tests work. The exact state-dependent Cut/Copy/Paste/Clear/Select-all classic popup, right-down/Apps-key triggers, retained logical focus with suppressed focus drawing, clipboard mutation rules, and activation-release capture are covered. Remaining: non-Windows middle-click primary selection and the exact zoomed `¦` caret glyph. |
| Description `TextWindow` scrolling | `C4GUI::TextWindow`, `ScrollWindow` | **Complete** | Wheel, clipping, fixed pin, track jump-and-drag with capture, and held arrows match the C++ geometry/conversions. |
| Scenario list scrolling | `C4GUI::ListBox` | **Complete** | Selection-follow viewport, wheel, clipping, fixed pin, scrolled clicks, captured track drag, held arrows, end-stopping Up/Down, and fully-visible-row PageUp/PageDown/Home/End are covered. |
| Choose Definitions checkbox and definition selector | `StartScenario`, `C4DefinitionSelDlg`; `lc-app/src/main.rs::PendingDefinitionSelection` | **Complete** | Selection/reset rules, LocalOnly, recursive focus/input, raw-order `*.c4d`, fixed/optional checks, exact dialog/list/preview/buttons, scrolling/title drag, F5 rebuild, nested error, modal/capture cleanup, cancel retention, ordered output and rooted loading are covered. Local versus `NetworkHost` mode survives refresh, cancel, error and accept. |
| Scenario rename | `ScenListItem::KeyRename`, `Entry::RenameTo` | **Missing** | Inline edit and all failure dialogs. |
| Scenario delete | `KeyDelete`, `DeleteConfirm` | **Missing** | Original warning, confirmation, deletion and errors. |
| Mission access/password | `KeyCheat`, `KeyCheat2` | **Missing** | Input modal and module add/remove. |
| Start validation/warnings | `Scenario::CanOpen`, `DoOK` | **Missing** | Access, replay/network, player limits, dedicated warnings. |
| Local Fair Crew/Record option buttons | `C4GameOptionButtons`; `lc-frontend/src/game_option_buttons.rs` | **Complete** | Exact strip geometry/resources, tooltips, focus, keyboard/pointer/touch/gamepad input, config persistence and `ForcedNoCrew` constraints are app-wired. |
| Network scenario book | `C4StartupScenSelDlg(true)`; `ScenarioSelectorMode::NetworkHost` | **Partial** | Network Create Game opens the selector before binding a socket. Recursive focus, touch search/list/open/back, modal ownership, definition-return state and close-surviving input capture are complete within the shared book. FolderMap, rename/delete, access and validation gaps remain. |
| Network selector option buttons | `C4GameOptionButtons`; `lc-frontend/src/game_option_buttons.rs` | **Complete** | Internet, League, Password, Comment, Fair Crew and Record use classic layouts/states/tooltips and full recursive keyboard, pointer, touch and gamepad routing. Boundary pass-through, per-gesture capture and resize cancellation are tested; values load/persist through config. This completion is scoped to the selector, not the unwired lobby. |
| Password/Comment `InputDialog` from network selector | `C4GameOptionButtons::OnBtnPassword`, `OnBtnComment`; `lc-frontend/src/input_dialog.rs` | **Complete** | Both selector callers use the resource-validating classic modal with edit/caret/selection/context actions, max length and all input modes. Strict underlying-screen exclusion, Apps-key context ownership, per-button gesture capture, close-surviving release consumption and resize cancellation are tested. Other `C4GUI::InputDialog` consumers remain Partial below. |

## Startup network browser and IRC subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Network browser chrome | `C4StartupNetDlg` | **Partial** | Static classic shell/controller exists. |
| Retained NetDlg Internet refresh | `C4StartupNetDlg::OnShown`, `UpdateMasterserver`; `NetDlgController::sync_masterserver_signup_from_config` | **Complete** | Returning from the host selector updates only the Internet icon/config in place. Active Chat/GameList mode, focus, join address, Record staleness and even held pointer/key latches remain oracle-faithful and tested. |
| Live game list/query entries | `C4StartupNetListEntry`, `UpdateList` | **Missing** | Masterserver/LAN/direct queries, references, status, errors, refresh throttling. |
| Direct-address two-stage query/join | `C4StartupNetDlg::DoOK` | **Partial** | Rust currently joins immediately. |
| Join validation/redirect/discovery modals | `DoOK`, `DoRefresh` | **Partial** | Empty selection now opens the exact classic `Cannot join game` OK/Error dialog. Bad references, version checks, redirect and discovery/error paths remain. |
| Create Game transition | `C4StartupNetDlg::CreateGame`; `lc-app/src/main.rs::process_network_dialog_actions` | **Complete** | It opens `C4StartupScenSelDlg(true)`/`NetworkHost` before any host socket or `NetworkManager` exists. |
| Selected host scenario/definitions staging | `C4StartupScenSelDlg::StartScenario`; `StagedNetworkHostScenario` | **Partial** | Scenario, ordered definitions and an immutable accepted option snapshot are retained before asynchronous host activation. Once accepted, the transition is noninteractive across keyboard, pointer, touch, gamepad and programmatic actions. League/start validation and the classic start-wait/lobby handoff remain. |
| Post-connect lobby boundary | `C4Game::NetworkJoin`, `C4GameLobby`; `GameApp::poll_startup_network_connection` | **Fail-fast** | Host and join completion log an error, refuse `NetworkLobbyState`, and immediately drop the successfully created manager/listener plus mode. The retained startup screen becomes interactive only after the async transition resolves. Do not infer lobby parity from this tested guard. |
| IRC login sheet | `C4ChatControl` | **Fail-fast** | Nick/password/name/channel/connect/disclaimer/errors. |
| IRC server/channel/query tabs | `C4ChatDlg`, `C4ChatControl` | **Missing** | Logs, input/history, nick lists, unread/query state. |
| IRC command language and quit confirmation | `C4ChatControl::ProcessInput` | **Missing** | Full command dispatch and confirmation. |

## Startup player subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player list/activation/info/portrait | `C4StartupPlrSelDlg` | **Partial** | Classic visual/controller and activation persistence exist. |
| Player row Properties/Delete context | `PlayerListItem::ContextMenu` | **Partial** | Whole-row right-down now opens the exact two-entry classic popup with no initial selection, focus suppression, tooltips, pointer/keyboard/touch/gamepad routing, outside-down pass-through and Delete activation into the exact confirmation chain. Properties logs and reports the unported form instead of opening a generic pane. |
| New Player / Properties form | `C4StartupPlrPropertiesDlg` | **Missing** | Current actions are status-only (Properties may log) rather than a consistent fail-fast boundary. Implement name, colors, control set, mouse, movement mode, portrait and validation, or return a logged error. |
| Portrait selector | `C4PortraitSelDlg` | **Missing** | Location combo, image grid, flags, OK/Cancel. |
| Crew mode and crew detail list | `SetCrewMode`, crew list classes | **Missing** | Current route is status-only. Implement participation, stats, portraits and sorting, or return a logged error. |
| Crew Rename/Delete/Death Message context | crew item callbacks | **Missing** | Inline rename, input modal, confirmation/errors. |
| Player/crew delete and validation dialogs | `OnDelBtn`, property close paths | **Partial** | Player Delete button/key/context now use the exact Yes/No warning (including the strict >10-hour suffix), permanently remove packed or directory `.c4p` groups, always rebuild list/selection/Participants, and show the exact failure dialog. Crew deletion and property validation remain. |

## Startup options subtree

Only the Program sheet may currently render. Other sheets return a logged
error instead of showing blank or generic panes.

| Sheet or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Options chrome/tabs | `C4StartupOptionsDlg` | **Complete** | Preserve layout/input snapshots and the fail-fast boundary for unsupported sheets. |
| Program | program sheet controls | **Partial** | Timestamp is live; language/font/chat/preload/fair-crew/reset/advanced are inert or missing. |
| Graphics | graphics sheet controls | **Fail-fast** | Display mode, scale/test, toggles, smoke/fire controls. |
| Sound | sound sheet controls | **Fail-fast** | FE/game music/sound and sliders/F3. |
| Keyboard | `ControlConfigArea` | **Fail-fast** | Remove generic ControlOptions pane; port selector, 12 keys, reset. |
| Key capture modal | `KeySelDialog` | **Missing** | Classic modal and device input semantics. |
| Gamepad | `ControlConfigArea` | **Fail-fast** | Device selector, 12 bindings, menu control, reset/open. |
| Network | network sheet controls | **Fail-fast** | Ports, masterserver, updates, UPnP, machine/nick. |
| Reset confirmation/quit | `OnResetConfigBtn` | **Missing** | Confirmation and application exit. |
| Advanced warning/config editor | `C4StartupOptionsAdvancedConfigDialog` | **Missing** | Dynamic section tabs and typed controls. |
| Timed scale confirmation | `ResChangeConfirmDlg` | **Missing** | 12-second Yes/No and automatic restore. |
| Validation/restart/font errors | options callbacks | **Missing** | Classic messages. |

## About/update subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Credits page/chrome | `C4StartupAboutDlg` | **Partial** | Strong static render; developer TextWindow scrolling is missing. |
| Licenses list/text page | `LicenseWindow` | **Fail-fast** | Selectable license list and scrollable contents. |
| Update button and full updater | `C4UpdateDlg`, `C4DownloadDlg` | **Missing** | Rust currently changes status text only; that is not fail-fast. Implement lookup/wait, redirect, available/no-update/error, download/apply/log, or return a logged error. |

## Network start/lobby subtree

The app deliberately rejects its old generic lobby after host/join connection.
`lc-frontend/src/game_lobby.rs` is a resource-validating implementation of the
initial Players-sheet slice, but it is only exported by `lc-frontend`; it is not
constructed, rendered, or serviced by `lc-app`. Component-only rows below are
therefore Partial, never app-level completion.

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Initial network start wait | `C4Network2Dialogs.cpp:552-586`, `C4Network2StartWaitDlg` | **Missing** | Joining-client list/status, Restart and Cancel. |
| Password challenge and connection progress | `C4Network2Dialogs`, network join callbacks | **Missing** | Password retry/cancel, wait/progress/error and return-to-browser transitions. |
| Host/join app entry | `C4GameLobby::MainDlg`; `GameApp::poll_startup_network_connection` | **Fail-fast** | The app logs, refuses the generic lobby and tears down the just-created manager. Wire the exact frontend only after all required actions have real owners. |
| Legacy generic `NetworkLobbyState` pane | no C++ visual authority | **Fail-fast** | CLI state may still be constructed, but render/completion guards prevent the generic overlay from becoming a menu. Remove it when the classic lobby owns the route. |
| Exact lobby frontend resource boundary | `C4GameLobby.cpp:141-314`; `lc-frontend/src/game_lobby.rs::LobbyResources` | **Partial** | The component validates every required classic image sheet/font and refuses substitutes, but it is not app-wired and renders only the initially visible Players slice. |
| Scenario/team/parameter metadata projection | `C4Scenario`, `C4TeamList`, `C4GameParameters`, `StdCompilerINIRead`; `lc-engine/src/scenario.rs` | **Partial** | Exact recursive INI hierarchy, case/duplicate/default/string semantics, compound-array recovery, teams, clients, savegame/replay rules and definition-resolution boundaries are projected and tested, including shipped content. Runtime-selected loader resources, current roster/script-player naming, config/network/league adjustments and old-save effective definition resources still require app ownership. |
| `PID_Status` / `PID_StatusAck` barrier foundation | `C4Network2Status`, `C4Network2::ChangeGameStatus`; `StatusBarrier`, `HostHandle`, `ClientHandle`, `NetworkManager` | **Partial** | Exact signed payloads, host barrier transitions, stale/duplicate rejection, `host_change_status`/`host_status_reached`, client acknowledgement, app events and real-TCP tests exist. The unwired lobby does not initiate or consume its status lifecycle. |
| `PID_LobbyCountdown` transport foundation | `C4PacketCountdown`, `Countdown`; `LobbyCountdown`, host/client session APIs | **Partial** | Exact codec, abort value, timer cadence predicate, host-only broadcast, client/app events, identity rules and real-TCP round trips are tested. No app lobby owns the countdown timer or applies the events to UI/start state. |
| `PID_ReadyCheck` / lobby-ready foundation | `C4PacketReadyCheck`; `ReadyCheck`, `HostHandle::set_lobby_ready`, `ClientHandle::set_lobby_ready`, `NetworkManager::set_local_ready` | **Partial** | Request/reply codec, role-correct host/client APIs, origin validation, relay rules, bounded app telemetry and real-TCP tests exist. No app lobby binds them to its checkbox, roster or ready-check modal. |
| Main transparent chrome and responsive layout | `C4GameLobby::MainDlg::MainDlg`; `game_lobby.rs` | **Partial** | Title, chat area, right caption, tab icons, bottom strip and small/normal geometry exist in the component; app lifecycle/background/focus is absent. |
| Chat log, scrolling and input edit | `C4GameLobby.cpp:272-280,475-766,1018-1056` | **Partial** | Component UI, scrolling, focus and edit requests exist; paste/history and synchronized colored log ownership are not app-wired. Transport cannot send chat yet because `CID_Message` is absent. |
| Lobby chat `C4ControlMessage` / `CID_Message` | `C4ControlMessage`, `CID_Message`; `lc-app/src/network.rs` | **Missing** | There is intentionally no opaque placeholder. Implement the conditional private-recipient codec plus authoritative player/team ownership and visibility validation before connecting chat input. |
| Chat input context popup | `C4GUI::Edit::OnContext`; `LobbyChatRequest::OpenContextMenu` | **Partial** | Component emits typed context requests; app must open and dispatch Cut/Copy/Paste/Clear/Select-all using the shared classic menu. |
| Exit and abort confirmation | `MainDlg::OnExitBtn`, `OnClosed` | **Partial** | Component emits Exit; classic confirmation, network teardown and return transition are missing. |
| Ready checkbox/loading lock | `MainDlg::OnReadyCheck`, `UpdatePreloadingGUIState`, `RequestReadyCheck` | **Partial** | Visual/input state and tested host/client ready APIs exist separately. App wiring to `set_local_ready`/events, authoritative roster state, cooldown, forced reset and preload gating is missing. |
| Host Start/Cancel and synchronized countdown | `MainDlg::OnRunBtn`, `Start`, `OnCountdownPacket`; `Countdown` | **Partial** | Component phases/locking and tested `PID_LobbyCountdown` transport exist separately. Wire validation, timer cadence, broadcast/event application, abort, auto-start, status transition and warnings into the app lobby. |
| Bottom lobby game-option strip | `C4GameOptionButtons`; `game_option_buttons.rs` | **Partial** | Exact Host/Client contexts, focus and countdown locks exist as a reusable component. Lobby Internet/League/Password/Comment/Fair Crew/Record controls still lack app/network dispatch. |
| Password and Comment lobby input dialogs | `C4Network2Dialogs.cpp:718-776` | **Partial** | Exact InputDialog and selector callbacks exist, but the lobby callers and network password/comment mutation are not wired. |
| Right tab icon buttons | `C4GameLobby.cpp:255-264,922-998` | **Partial** | Players/Teams/Resources/Options/Scenario/IRC requests are emitted; only Players content is rendered. |
| Right-caption tab context popup | `MainDlg::OnRightTabContext` (`C4GameLobby.cpp:844-866`) | **Missing** | This C++ popup is flat—Players, optional Teams, Resources, Options—not recursive. The component only emits `TabContextRequested`; Scenario remains a direct tab button. |
| Players/clients roster layout and scrolling | `C4PlayerInfoListBox.cpp:39-488,1237-1607`; `game_lobby.rs` | **Partial** | Component renders the visible client/player hierarchy, state icons, selection, focus and scroll input. Live model updates and all requested child actions need app owners. |
| Player row context root | `C4PlayerInfoListBox.cpp:490-533` | **Missing** | Conditional Take Over, Remove and New Color entries with exact permissions/tooltips. |
| Nested Take Over submenu | `C4PlayerInfoListBox.cpp:535-572` | **Missing** | Recursively enumerate free savegame players and dispatch takeover by player ID. |
| Player Remove confirmation/control | `C4PlayerInfoListBox.cpp:574-600` | **Missing** | Host/control authorization, confirmation/error and synchronized removal. |
| Player New Color action | `C4PlayerInfoListBox.cpp:602-616` | **Missing** | Generate and send the synchronized player-color change. |
| Player team combo/dropdown | `C4PlayerInfoListBox.cpp:618-646` | **Partial** | Component exposes team selection requests and row affordance; classic ComboBox popup, permission/filter rules and synchronized team change are missing. |
| Client rows/status/ping/sound/add button | `C4PlayerInfoListBox.cpp:737-916` | **Partial** | Component draws bounded row variants and emits Add Player/context requests. Ready/status events and APIs now exist at transport/app boundaries, but no lobby model applies them; live ping/sound updates and commands also remain. |
| Client context Mute/Kick/Activate/Info | `C4PlayerInfoListBox.cpp:918-979` | **Missing** | Flat conditional popup, kick/vote confirmation, activation control, mute state and detail dialog. |
| Add Player selector | `C4PlayerInfoListBox.cpp:981-985`, `MainDlg::OnClientAddPlayer` | **Missing** | Player-file selector, target-client handoff, countdown abort and errors. |
| Team header rows and move-local-players action | `C4PlayerInfoListBox.cpp:989-1110` | **Partial** | Component has team row presentation/request types; team-filtered roster construction and synchronized bulk move remain. |
| Free-savegame player group | `C4PlayerInfoListBox.cpp:1112-1143` | **Partial** | Component can present the group; takeover context and restoration are missing. |
| Script-player group and Add action | `C4PlayerInfoListBox.cpp:1146-1210` | **Partial** | Component can present/add-request the row; max-player gating and synchronized script-player creation are missing. |
| Replay-player group | `C4PlayerInfoListBox.cpp:1212-1235` | **Partial** | Component presentation exists; replay-specific state/update ownership is absent. |
| Teams-filtered sidebar mode | `MainDlg::OnTabTeams`, `C4PlayerInfoListBox::PILBM_LobbyTeamSort` | **Missing** | Build the filtered/team-sorted sheet and preserve tab/focus/selection state. |
| Resources list/progress | `C4Network2ResDlg.cpp:32-87,158-216` | **Missing** | Live resources, status icons/progress, activation timer and scrolling. |
| Resource Save/overwrite/success/error branch | `C4Network2ResDlg.cpp:88-157` | **Missing** | Save button, path handling, overwrite confirmation and completion/error dialogs. |
| Preload button, automatic preload and failure log | `C4GameLobby.cpp:231-245,766-842,1001-1016` | **Missing** | Readiness gating, automatic/manual preload and red failure logging. |
| Options list container | `C4GameOptions.cpp:28-310` | **Missing** | List layout, one-second refresh and ComboBox plumbing. |
| Options / Control mode | `C4GameOptionsList::OptionControlMode` | **Missing** | Central/decentral/async choices and host/read-only rules. |
| Options / Control rate | `OptionControlRate` | **Missing** | Values 1–9, synchronized adjustment and refresh. |
| Options / Runtime join | `OptionRuntimeJoin` | **Missing** | Barred/free choices, config persistence and live allow-join. |
| Options / Team distribution | `OptionTeamDist` | **Missing** | Scenario-provided choices and synchronized selection. |
| Options / Team colors | `OptionTeamColors` | **Missing** | Enabled/disabled choice and synchronized update. |
| Options / Random team count | `OptionRandomTeamCount` | **Missing** | Conditional values, team recreation and dependent option refresh. |
| Scenario-description sheet | `C4GameLobby.cpp:64-137`, `ScenDesc` | **Missing** | Incremental load, markup text, portrait/preview behavior and scrolling. |
| Optional IRC/chat window button | `MainDlg::OnBtnChat`, `C4ChatDlg` | **Missing** | Open/raise the full IRC dialog without conflating it with lobby chat. |
| Per-client information dialog | `C4Network2Dialogs.cpp:40-289`, `C4Network2ClientDlg` | **Missing** | IDs, addresses, connection list, status, ping and host actions. |

## In-game main-menu subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player/observer main page | `C4MainMenu::ActivateMain` | **Partial** | Condition data is hardcoded; several child actions missing. |
| Goals list | `ActivateGoals` | **Partial** | Fulfilled star/description and goal-info screen. |
| Rules list | `ActivateRules` | **Partial** | Rule-info screen. |
| Hostility | `ActivateHostility` | **Missing** | Player list, hostility toggle/control. |
| Initial team selection/team switch | `C4Player::ActivateMenuTeamSelection` | **Missing** | `TeamInfo` now retains all 39 recursively inventoried `IconSpec` recipes; render team rows, availability, queued selection and view preview. |
| Observer target/free view | `ActivateObserver` | **Missing** | View entries and camera transition. |
| Runtime player join | `ActivateNewPlayer` | **Partial** | Page constructor exists; discovery/join action missing. |
| Ten save slots | `ActivateSavegame` | **Partial** | Functional page; grouped naming/path/localization semantics differ. |
| Options | `ActivateOptions` | **Partial** | Visible toggles exist; config/localization persistence incomplete. |
| Display | `ActivateDisplay` | **Partial** | Several toggles are state-only and do not affect rendering. |
| Host disconnect/client kick | `ActivateHost` | **Missing** | Client list and removal controls. |
| Client disconnect | `ActivateClient` | **Partial** | Yes/No page exists; Part/network teardown missing. |
| Surrender | `ActivateSurrender` | **Partial** | Offline works; queued/network/league semantics missing. |
| Abort/restart/no | `C4GameDialogs.cpp:33-128`, `C4AbortGameDialog` | **Fail-fast** | The menu-shaped approximation is no longer reachable: activation logs and returns a typed error. Exact halt state, restart policy and cancel semantics remain unported. |
| Mouse hit testing/scrollbars/tooltips | `C4Menu` | **Partial** | Keyboard navigation works; pointer/scroll behavior incomplete. |
| In-game/script menu font and sheet boundary | `C4GUI::Resource`, `C4Menu::Draw`; `lc-frontend/src/ingame_menu.rs`, `lc-app/src/main.rs` | **Fail-fast** | Visible in-game/script menus preflight the exact fonts and sheets and return a typed, logged error before generic frames/fonts can draw. This does not yet cover every non-menu HUD fallback. |
| HUD resource boundary | `C4GraphicsResource`, player HUD; `lc-frontend/src/hud.rs` | **Missing** | The Rust HUD can synthesize fallback surfaces. Replace that reachability with a logged error until classic resources load. |

## Object-menu subtree

Generic app-owned inventory/get/build panes are rejected at render time.

| Menu identification/style | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Construction (`C4MN_Construction`) | `C4ObjectMenu::Refill` | **Fail-fast** | Knowledge/component-based classic menu and Construct command. |
| Activate (`C4MN_Activate`) | `C4ObjectMenu` | **Fail-fast** | Classic inventory activation rows/commands. |
| Get (`C4MN_Get`) | `C4ObjectMenu` | **Fail-fast** | Classic nearby/container get menu. |
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
| Selection/close/command callbacks | `C4Menu`, script callbacks | **Partial** | Scenario callbacks and MenuQueryCancel gaps. |
| Script-menu resource/font fallback boundary | `C4Menu::Draw`, `C4GUI::Resource`; `ingame_menu.rs` | **Fail-fast** | App preflight prevents every visible style from reaching generic fonts, frames or sheets; exact resources are still required before these Partial renderers can become Complete. |

## Other game-visible dialogs and overlays

| Screen | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Game over/evaluation | `C4GameOverDlg` | **Partial** | Custom evaluation, league/network results, pending stream, icons; missing resources fail-fast. |
| Game-over Enter/chat behavior | `C4GameOverDlg.cpp:317-321` | **Missing** | C++ Enter opens chat; Rust activates the selected evaluation control. Restore the chat binding and its recursive input screen. |
| In-game chat/script query input | `C4MessageInput` | **Missing** | Modes/history/completion/paste/team say/script query; network chat also awaits the safe `CID_Message` codec and roster validation. |
| Scoreboard dialog | `C4ScoreboardDlg` | **Missing** | Engine data exists; render/input absent. |
| Loader/progress/log screen | `C4LoaderScreen.cpp:126-177`; `lc-frontend/src/loader_screen.rs` | **Partial** | An exact, resource-validating frontend models logical chrome, native-scale text, title/progress/log/process state, GUI fallback, filtering, texture tiling/padding and resource refresh. Loader discovery/selection, decode, live boot/scenario callbacks and the two-pass app render are not wired. |
| Loader generic-fallback boundary | `C4LoaderScreen`; `lc-app/src/main.rs::render_loading` | **Fail-fast** | Boot/scenario loading logs and returns a typed error instead of drawing the old generic pane. Replace this temporary boundary only when the exact frontend is fully app-wired. |
| Object/global game messages (`C4GameMessage`) | `C4GameMessage.cpp:38-231` | **Fail-fast** | Every drawable local/observer viewport is checked, including duplicate-owner split views and C++ parallax projection. Drawable messages return a typed, logged boundary; target visibility/FoW snapshots that cannot be classified exactly use a separate fail-closed boundary. Remote-only, missing-target and logically offscreen messages do not overblock. The exact renderer remains unported. |
| Message board modes/history | `C4MessageBoard` | **Partial** | Only one current line; multiline/history/scroll missing. |
| Runtime client/connection dialog | `C4Network2ClientDlg` | **Missing** | F4 list/actions/options. |
| Runtime client list | `C4Network2ClientListDlg` | **Missing** | F4 list rows, selection and actions are distinct from the per-client detail dialog. |
| Runtime ready-check timed dialog/toast | `C4Network2::ReadyCheckDialog`, `C4Network2.cpp:129-198,1625-1690` | **Missing** | `PID_ReadyCheck` transport and ready APIs are Partial above, but this screen is absent: participant state text, timeout, toast action and app response binding remain. |
| Network vote dialog | `C4VoteDialog`, `C4Network2.cpp:2941-3014` | **Missing** | Vote queue text, Yes/No dispatch, replacement/close ownership and result updates. |
| League surrender confirmation | `C4Network2::OpenSurrenderDialog`, `LeagueSurrender` | **Missing** | Forfeit-specific confirmation and disconnect/report behavior. |
| Network statistics chart | `C4Network2Stats` dialog | **Missing** | OC/FPS/IO/ping/control/APM tabs. |
| League signup/register/auth form | `C4LeagueSignupDialog`, `C4League.cpp:548-749` | **Missing** | Account/password/confirm/check-password fields, validation, warning and modal return. |
| League waits/retries/results | `C4League`, network dialogs | **Missing** | Request progress, retries, authentication failures, score/rank/result screens and disconnect reporting. |
| Resource download progress | `C4DownloadDlg`, network resource UI | **Missing** | Progress/cancel/errors. |
| File/player/definition/portrait selectors | `C4FileSelDlg` family | **Partial** | The multi-select definition specialization is complete, including its recursive error modal and runtime handoff. Single-file, player and portrait specializations, locations/combo boxes and portrait options remain. |
| Rust-only F5 quick save | no C++ menu equivalent; C++ uses `C4MainMenu::ActivateSavegame` | **Fail-fast** | The shortcut logs and returns a typed boundary instead of bypassing the classic ten-slot flow. |
| Rust-only F9 quick load | conflicts with C++ `C4Game.cpp:3373-3374` | **Fail-fast** | The shortcut no longer quick-loads; it logs and returns a typed boundary. Restore the C++ screenshot action and keep loading in an oracle-backed menu. |
| C++ F9 screenshot feedback/path | `C4Game.cpp:3373-3374` | **Missing** | Capture, filename/error and user feedback are not represented by the Rust quick-load action. |
| Rust-only F6/F7 named save/load browser | no C++ equivalent; `lc-frontend/src/save_browser.rs` | **Fail-fast** | The renderer remains in-tree, but the app guard rejects both shortcuts. Remove it or replace it with the classic ten-slot flow; never relax the guard. |
| Rust synthetic menu sandbox | no C++ player-facing equivalent; `lc-app/src/main.rs` | **Complete** | Empty discovery/catalog paths no longer inject the sandbox into player-facing menus. The explicit `--sandbox` developer/test route remains intentionally separate from menu discovery. |

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
6. recursive scenario/script corpus coverage where the screen is data-driven.
