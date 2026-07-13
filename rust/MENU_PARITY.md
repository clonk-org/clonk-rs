# C++ Menu Parity Inventory

This is the recursive completion checklist for the Rust menu port. The C++
classes in `../src/` are authoritative. A top-level screen is not complete
until every child sheet, context menu, confirmation/input dialog, callback,
scroll path, and transition reachable from it is complete.

Status meanings:

- **Parity**: C++ behavior and presentation are covered by executable evidence.
- **Partial**: a classic-shaped implementation exists, but listed behavior is
  missing or indirect.
- **Fail-fast**: Rust logs and returns an error instead of drawing a generic or
  incomplete pane. This is preferable to false parity, but is not completion.
- **Missing**: no usable Rust implementation exists.

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
| Main six-button screen | `C4StartupMainDlg` | **Parity/strong** | Keep pixel/input snapshots current. |
| Dialog switch/fade/back stack | `C4Startup::SwitchDialog`, `C4StartupDlg::OnClosed` | **Partial** | Match fade timing, previous-dialog ownership and every Escape/back transition; Rust currently switches a flat `StartupView`. |
| Common startup cursor/focus/hotkeys/tooltips | `C4GUI::Screen`, `Dialog`, `Control` | **Partial** | Centralize the classic cursor, 500 ms tooltip delay, Tab/Shift-Tab traversal and Alt hotkeys instead of per-screen approximations. |
| Participants Add/Remove context and player submenu | `C4StartupMainDlg::OnPlayerSelContext*` | **Parity/strong** | Exact autosized markup label target and tooltip, recursive lazy Add/Remove tree (including empty 40x7 children), raw filesystem/config ordering, raw Remove indices, player icons/tooltips, activation/validation/deduplication, focus suppression and fail-fast resources are covered. |
| First-run new-player properties modal | `C4StartupMainDlg::OnShown` | **Missing** | Depends on player-properties dialog. |
| Automatic/manual/incoming update flow | `C4StartupMainDlg::OnShown`, `C4UpdateDlg` | **Missing** | See update subtree. |
| F6 editor launch from startup | `C4StartupMainDlg::SwitchToEditor` | **Missing** | Bind and validate the legacy editor launch. |
| Dormant Rust editor scenario action | no C++ startup-list equivalent | **Fail-fast** | Remove or prove parity for `ScenarioKind::Editor` / `StartupMenuAction::EditEntry`; it must not expose a Rust-only menu route. |
| Startup fatal/restart log info dialog | `C4Startup::DoStartup` | **Missing** | Classic expandable info dialog. |

## Scenario selection subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Local scenario book chrome/list/info/buttons | `C4StartupScenSelDlg` | **Partial** | Exact focus, name hotkeys, refresh, rename/delete. |
| Recursive `.c4s`/`.c4f`/directory discovery | `C4ScenarioListLoader` | **Parity/strong** | Retain recursive discovery/order tests. |
| Folder navigation and folder metadata | `Folder::Start`, `FolderBack` | **Partial** | Runtime reload/mutation parity. |
| Folder-map view (`FolderMap.txt`) | `C4MapFolderData` | **Missing** | Background, scenario buttons, overlays, access graphics, map info pane. |
| Search edit/filter | `OnSearchBarEnter`, `UpdateList`, `KeySearch`, `C4GUI::Edit` | **Partial** | Submit-only markup-stripped filtering, Ctrl+F select-all, caret/selection, word edits, clipboard shortcuts, mouse capture/double-click, horizontal scroll, blink, and render tests work. The exact state-dependent Cut/Copy/Paste/Clear/Select-all classic popup, right-down/Apps-key triggers, retained logical focus with suppressed focus drawing, clipboard mutation rules, and activation-release capture are covered. Remaining: non-Windows middle-click primary selection and the exact zoomed `¦` caret glyph. |
| Description `TextWindow` scrolling | `C4GUI::TextWindow`, `ScrollWindow` | **Parity/strong** | Wheel, clipping, fixed pin, track jump-and-drag with capture, and held arrows match the C++ geometry/conversions. |
| Scenario list scrolling | `C4GUI::ListBox` | **Parity/strong** | Selection-follow viewport, wheel, clipping, fixed pin, scrolled clicks, captured track drag, held arrows, end-stopping Up/Down, and fully-visible-row PageUp/PageDown/Home/End are covered. |
| Choose Definitions checkbox | `StartScenario`, `C4DefinitionSelDlg` | **Partial** | The local selector covers selection-reset rules, the disabled-but-checked LocalOnly edge case, recursive row-checkbox focus, Alt+D/Tab/Space/pointer/gamepad input with close-surviving release capture, flat raw-order `*.c4d` enumeration, fixed/optional checks, exact dialog/list/preview/buttons, scrolling/title drag, F5 rebuild quirk, nested selection error, cancel retention, ordered fixed output, strict rooted-plus-original DefinitionPath loading, Restart/Next Mission and exact-save vector retention, and fail-fast resources. Remaining: route network Create Game through `C4StartupScenSelDlg(true)` instead of directly into the lobby. |
| Scenario rename | `ScenListItem::KeyRename`, `Entry::RenameTo` | **Missing** | Inline edit and all failure dialogs. |
| Scenario delete | `KeyDelete`, `DeleteConfirm` | **Missing** | Original warning, confirmation, deletion and errors. |
| Mission access/password | `KeyCheat`, `KeyCheat2` | **Missing** | Input modal and module add/remove. |
| Start validation/warnings | `Scenario::CanOpen`, `DoOK` | **Missing** | Access, replay/network, player limits, dedicated warnings. |
| Fair Crew/Record option buttons | `C4GameOptionButtons` | **Partial** | Icons render; pointer/action/config persistence missing. |
| Network scenario book | `C4StartupScenSelDlg(true)` | **Missing** | Restore Network New Game -> network scenario selection before lobby. |
| Network option buttons | `C4GameOptionButtons` | **Missing** | Internet, League, Password, Comment, Fair Crew, Record. |

## Startup network browser and IRC subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Network browser chrome | `C4StartupNetDlg` | **Partial** | Static classic shell/controller exists. |
| Live game list/query entries | `C4StartupNetListEntry`, `UpdateList` | **Missing** | Masterserver/LAN/direct queries, references, status, errors, refresh throttling. |
| Direct-address two-stage query/join | `C4StartupNetDlg::DoOK` | **Partial** | Rust currently joins immediately. |
| Join validation/redirect/discovery modals | `DoOK`, `DoRefresh` | **Partial** | Empty selection now opens the exact classic `Cannot join game` OK/Error dialog. Bad references, version checks, redirect and discovery/error paths remain. |
| Create Game transition | `C4StartupNetDlg::CreateGame` | **Missing** | Must open network scenario book, not generic lobby. |
| IRC login sheet | `C4ChatControl` | **Fail-fast** | Nick/password/name/channel/connect/disclaimer/errors. |
| IRC server/channel/query tabs | `C4ChatDlg`, `C4ChatControl` | **Missing** | Logs, input/history, nick lists, unread/query state. |
| IRC command language and quit confirmation | `C4ChatControl::ProcessInput` | **Missing** | Full command dispatch and confirmation. |

## Startup player subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player list/activation/info/portrait | `C4StartupPlrSelDlg` | **Partial** | Classic visual/controller and activation persistence exist. |
| Player row Properties/Delete context | `PlayerListItem::ContextMenu` | **Partial** | Whole-row right-down now opens the exact two-entry classic popup with no initial selection, focus suppression, tooltips, pointer/keyboard/touch/gamepad routing, outside-down pass-through and Delete activation into the exact confirmation chain. Properties logs and reports the unported form instead of opening a generic pane. |
| New Player / Properties form | `C4StartupPlrPropertiesDlg` | **Missing** | Name, colors, control set, mouse, movement mode, portrait, validation. |
| Portrait selector | `C4PortraitSelDlg` | **Missing** | Location combo, image grid, flags, OK/Cancel. |
| Crew mode and crew detail list | `SetCrewMode`, crew list classes | **Missing** | Participation, stats, portraits, sorting. |
| Crew Rename/Delete/Death Message context | crew item callbacks | **Missing** | Inline rename, input modal, confirmation/errors. |
| Player/crew delete and validation dialogs | `OnDelBtn`, property close paths | **Partial** | Player Delete button/key/context now use the exact Yes/No warning (including the strict >10-hour suffix), permanently remove packed or directory `.c4p` groups, always rebuild list/selection/Participants, and show the exact failure dialog. Crew deletion and property validation remain. |

## Startup options subtree

Only the Program sheet may currently render. Other sheets return a logged
error instead of showing blank or generic panes.

| Sheet or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Options chrome/tabs | `C4StartupOptionsDlg` | **Parity/strong** | Preserve layout/input snapshots. |
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
| Update button and full updater | `C4UpdateDlg`, `C4DownloadDlg` | **Missing** | Lookup/wait, redirect, available/no-update/error, download/apply/log. |

## Network start/lobby subtree

The current generic Rust lobby is rejected at render time.

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Initial network start wait | `C4Network2StartWaitDlg` | **Missing** | Join client list, Restart, Cancel. |
| Lobby main/chat/input/ready/go/exit | `C4GameLobby::MainDlg` | **Fail-fast** | Full classic dialog and synchronized behavior. |
| Players/Teams sidebar | `C4PlayerInfoListBox` | **Missing** | Team moves, takeover, remove/recolor, client contexts. |
| Resources/Preload sidebar | `C4Network2ResDlg` | **Missing** | Progress, save, overwrite/success/error. |
| Options sidebar | `C4GameOptionsList` | **Missing** | Control/team/runtime-join options. |
| Scenario-description sidebar | `C4GameLobby::ScenDesc` | **Missing** | Loading and scrolling. |
| Lobby game-option buttons | `C4GameOptionButtons` | **Missing** | Internet/league/password/comment/fair crew/record. |
| Countdown/warnings/chat commands | `C4GameLobby` | **Missing** | Synchronized start and history. |
| Client info dialog | `C4Network2ClientDlg` | **Missing** | Status, IDs, addresses, connections, ping. |
| Right-tab context popup | `C4GameLobby::MainDlg::OnRightTabContext` | **Missing** | Recursive classic popup actions for lobby tabs. |

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
| Abort/restart/no | `C4AbortGameDialog` | **Partial** | Menu-shaped approximation; exact dialog and halt/cancel semantics missing. |
| Mouse hit testing/scrollbars/tooltips | `C4Menu` | **Partial** | Keyboard navigation works; pointer/scroll behavior incomplete. |

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
| EqualItemHeight flag | `C4MN_Style_EqualItemHeight` | **Parity/strong** | The raw style bit is preserved independently of base style and Dialog symbol rows use the C++ equalization/restacking rule. |
| Rank/Indexed/ObjRank/Object/TextSpec/Color symbols | `AddMenuItem` image grammar | **Partial** | Shipped calls use exact fallback-definition resolution, clipped indexed phases/colors, inherited rank strips and extended/captain ranks, and add-time Object/ObjectRank snapshots. Snapshots include temporary objects, foreign graphics/overlays/transforms, `ChangeDef`, color modulation, and Dialog's 64-pixel symbol size. All 75 shipped definition portraits are recursively inventoried/loaded in group order, and natural Western/LastWill paths are covered. TextSpec now shares one exact parser for uppercase/underscore IDs, scanf-style indexed phases, named portraits with real bitmap validation, and all seven prefix-matched `Ico:*` forms; inline images participate in title/body measurement, rendering, tooltips and pointer layout for every style, including Goldrush's shipped `SBTR` Context rows. Remaining shipped gap: persisted/current-vs-permanent crew portraits used by Knights/Hazard. Remaining mod-facing gaps include late-layout Context ObjectRank height, active-scenario graphics/font overrides, and owned-pixel snapshots if asset hot reload is introduced. |
| TextSpec grammar/corpus | `C4Game::DrawTextSpecImage`, `C4DefList::GetFontImage` | **Parity/strong** | Parser tables cover C scanf prefixes, raw portrait IDs/colors, case rules and icon prefixes; recursive shipped team census covers 39 `IconSpec` values in 19 files. Keep the Goldrush Context render/hit-test regression current. |
| Footer extras (`C4MN_Extra_*`) | `C4Menu::DrawElement` | **Partial** | Value, magic, components and close/command strips are modeled; retain explicit coverage for every extra kind and dynamic refill source. |
| Explicit size/permanence/alignment | `SetMenuSize`, `SetMenuPermanent`, `CreateMenu` alignment | **Partial** | Core fields survive; finish free-position Context requests, movement/clamping and close semantics. |
| Primary/secondary commands | `C4MenuItem::Command`, `Command2` | **Partial** | Engine preserves both command strings; complete right-click dispatch through every menu style and nested callback path. |
| Menu decoration | `SetMenuDecoration` | **Partial** | `SetByDef` snapshots callback-derived color/borders and all eight `FrameDeco*` facets immediately; every classic style applies packed alpha, tiled/truncated edges, protruding corners, clipped out-of-bounds source facets, and captured margins. Background-only decoration is valid; unsafe geometry and unresolved drawable facets fail fast. Real LastWill assets and the natural Western Goldrush dialogue path are covered. Remaining: definition hot-reload refresh/clear behavior when Rust gains that API. |
| Progressive text | `SetMenuTextProgress` | **Parity/strong** | Per-row byte offsets, portrait exclusion, markup-skipping shared budgets, selectable-row reveal, late-added rows, one-byte menu ticks, explicit show-text, local-only first-input conversion, and byte-prefix Dialog rendering are modeled and tested. |
| Selection/close/command callbacks | `C4Menu`, script callbacks | **Partial** | Scenario callbacks and MenuQueryCancel gaps. |

## Other game-visible dialogs and overlays

| Screen | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Game over/evaluation | `C4GameOverDlg` | **Partial** | Custom evaluation, league/network results, pending stream, icons; missing resources fail-fast. |
| In-game chat/script query input | `C4MessageInput` | **Missing** | Modes/history/completion/paste/team say/script query. |
| Scoreboard dialog | `C4ScoreboardDlg` | **Missing UI** | Engine data exists; render/input absent. |
| Loader/progress/log screen | `C4LoaderScreen` | **Partial** | Scenario title, log region, exact bar/process/copyright layout. |
| Message board modes/history | `C4MessageBoard` | **Partial** | Only one current line; multiline/history/scroll missing. |
| Runtime client/connection dialog | `C4Network2ClientDlg` | **Missing** | F4 list/actions/options. |
| Runtime client list | `C4Network2ClientListDlg` | **Missing** | F4 list rows, selection and actions are distinct from the per-client detail dialog. |
| Ready check/vote/league surrender | network dialog classes | **Missing** | Timed/synchronized modal behavior. |
| Network statistics chart | `C4Network2Stats` dialog | **Missing** | OC/FPS/IO/ping/control/APM tabs. |
| League signup/auth/results | `C4League`, network dialogs | **Missing** | Forms, waits, retries, confirmations, result screens. |
| Resource download progress | `C4DownloadDlg`, network resource UI | **Missing** | Progress/cancel/errors. |
| File/player/definition/portrait selectors | `C4FileSelDlg` family | **Partial** | The multi-select definition specialization is complete, including its recursive error modal and runtime handoff. Single-file, player and portrait specializations, locations/combo boxes and portrait options remain. |
| Rust-only named save/load browser | no C++ equivalent | **Fail-fast** | Remove it; use the classic ten-slot flow. |

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
- text/password input;
- timed confirmation;
- wait/progress with cancel;
- scrolling text/info;
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

- `C4Console` native menu bar and every File, Components, dynamic Player,
  dynamic Viewport, Network and Help branch, plus script entry.
- Scenario component Script/Title/Info editor.
- Landscape Drawing Tools (`C4ToolsDlg`).
- Object Properties/script input (`C4PropertyDlg`).
- Object List (`C4ObjectListDlg`).
- Shared developer notebook (`C4DevmodeDlg`), edit-cursor contexts, detached
  and new viewport windows.
- Platform-native open/save/message/crash dialogs.

These may be scheduled after player-facing menus, but literal full C++ GUI
parity is not achieved while they remain absent.

## Verification gates

Every row promoted to **Parity** needs:

1. a C++-referenced behavior test for each transition and nested action;
2. input coverage for keyboard, mouse, wheel/drag, touch where C++ supports it;
3. state restoration/focus/selection/scroll boundary coverage;
4. render evidence using real classic resources at audited resolutions/scales;
5. no reachable generic fallback;
6. recursive scenario/script corpus coverage where the screen is data-driven.
