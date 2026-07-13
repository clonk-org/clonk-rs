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
  `planet/**/*.c` files. Styles comprise Normal, Context, Info, and Dialog.
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
| Participants Add/Remove context and player submenu | `C4StartupMainDlg::OnPlayerSelContext*` | **Missing** | Context popup, nested player items, activation changes. |
| First-run new-player properties modal | `C4StartupMainDlg::OnShown` | **Missing** | Depends on player-properties dialog. |
| Automatic/manual/incoming update flow | `C4StartupMainDlg::OnShown`, `C4UpdateDlg` | **Missing** | See update subtree. |
| F6 editor launch from startup | `C4StartupMainDlg::SwitchToEditor` | **Missing** | Bind and validate the legacy editor launch. |
| Startup fatal/restart log info dialog | `C4Startup::DoStartup` | **Missing** | Classic expandable info dialog. |

## Scenario selection subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Local scenario book chrome/list/info/buttons | `C4StartupScenSelDlg` | **Partial** | Scrollbar arrows/thumb drag, exact focus, name hotkeys, refresh, rename/delete. |
| Recursive `.c4s`/`.c4f`/directory discovery | `C4ScenarioListLoader` | **Parity/strong** | Retain recursive discovery/order tests. |
| Folder navigation and folder metadata | `Folder::Start`, `FolderBack` | **Partial** | Runtime reload/mutation parity. |
| Folder-map view (`FolderMap.txt`) | `C4MapFolderData` | **Missing** | Background, scenario buttons, overlays, access graphics, map info pane. |
| Search edit/filter | `OnSearchBarEnter`, `UpdateList`, `KeySearch` | **Partial** | Ctrl+F/text/Enter and markup-stripped substring filtering work; add exact select-all/cursor/clipboard/edit scrolling and render snapshots. |
| Description `TextWindow` scrolling | `C4GUI::TextWindow`, `ScrollWindow` | **Partial** | Wheel scrolling, clipping, bounds, and pin rendering work; add arrows, track jumps, thumb drag/hold repeat. |
| Scenario list scrolling | `C4GUI::ListBox` | **Partial** | Selection-follow viewport, wheel, clipped render, pin, and scrolled click mapping work; add arrows, pin drag, PageUp/PageDown/Home/End. |
| Choose Definitions checkbox | `StartScenario` | **Missing** | Interaction plus `C4DefinitionSelDlg`. |
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
| Join validation/redirect/discovery modals | `DoOK`, `DoRefresh` | **Missing** | Classic reusable modal stack. |
| Create Game transition | `C4StartupNetDlg::CreateGame` | **Missing** | Must open network scenario book, not generic lobby. |
| IRC login sheet | `C4ChatControl` | **Fail-fast** | Nick/password/name/channel/connect/disclaimer/errors. |
| IRC server/channel/query tabs | `C4ChatDlg`, `C4ChatControl` | **Missing** | Logs, input/history, nick lists, unread/query state. |
| IRC command language and quit confirmation | `C4ChatControl::ProcessInput` | **Missing** | Full command dispatch and confirmation. |

## Startup player subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player list/activation/info/portrait | `C4StartupPlrSelDlg` | **Partial** | Classic visual/controller and activation persistence exist. |
| Player row Properties/Delete context | `PlayerListItem::ContextMenu` | **Missing** | Context menu and commands. |
| New Player / Properties form | `C4StartupPlrPropertiesDlg` | **Missing** | Name, colors, control set, mouse, movement mode, portrait, validation. |
| Portrait selector | `C4PortraitSelDlg` | **Missing** | Location combo, image grid, flags, OK/Cancel. |
| Crew mode and crew detail list | `SetCrewMode`, crew list classes | **Missing** | Participation, stats, portraits, sorting. |
| Crew Rename/Delete/Death Message context | crew item callbacks | **Missing** | Inline rename, input modal, confirmation/errors. |
| Player/crew delete and validation dialogs | `OnDelBtn`, property close paths | **Missing** | Classic modal/error behavior. |

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

## In-game main-menu subtree

| Screen or recursive child | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Player/observer main page | `C4MainMenu::ActivateMain` | **Partial** | Condition data is hardcoded; several child actions missing. |
| Goals list | `ActivateGoals` | **Partial** | Fulfilled star/description and goal-info screen. |
| Rules list | `ActivateRules` | **Partial** | Rule-info screen. |
| Hostility | `ActivateHostility` | **Missing** | Player list, hostility toggle/control. |
| Initial team selection/team switch | `C4Player::ActivateMenuTeamSelection` | **Missing** | Team rows, availability, queued selection, view preview. |
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
| Info (`C4MN_Info`) | `C4ObjectMenu` | **Fail-fast** | Classic Info layout and dynamic GetInfoString. |
| Contents (`C4MN_Contents`) | `C4ObjectMenu` | **Partial** | CollectionLimit/RejectCollection switching and exact refill. |
| Selection-follow scrolling/pointer drag | `C4Menu`, `C4ObjectMenu` | **Missing** | Scrollbars, wheel/thumb, construct drag. |

## Script-created menu grammar

| Feature | C++ authority | Rust status | Remaining work |
|---|---|---|---|
| Normal style | `CreateMenu`, `AddMenuItem` | **Partial** | Exact symbol recipes, scrollbars, decoration/progress. |
| Context style | same | **Partial** | Exact pointer/scroll semantics and symbols. |
| Info style | `C4MN_Style_Info` | **Fail-fast** | Classic layout/pointer behavior. |
| Dialog style | `C4MN_Style_Dialog` | **Fail-fast** | Classic layout/pointer behavior. |
| EqualItemHeight flag | `C4MN_Style_EqualItemHeight` | **Missing** | Preserve flag and layout rule. |
| Rank/Indexed/ObjRank/Object/TextSpec/Color symbols | `AddMenuItem` image grammar | **Missing** | Store and render all recipes. |
| Menu decoration | `SetMenuDecoration` | **Missing** | Shipped LastWill/Western dialogs use it. |
| Progressive text | `SetMenuTextProgress` | **Missing** | Advance/distribute/reveal on key; shipped dialogues use it. |
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
| Ready check/vote/league surrender | network dialog classes | **Missing** | Timed/synchronized modal behavior. |
| Network statistics chart | `C4Network2Stats` dialog | **Missing** | OC/FPS/IO/ping/control/APM tabs. |
| League signup/auth/results | `C4League`, network dialogs | **Missing** | Forms, waits, retries, confirmations, result screens. |
| Resource download progress | `C4DownloadDlg`, network resource UI | **Missing** | Progress/cancel/errors. |
| File/player/definition/portrait selectors | `C4FileSelDlg` family | **Missing** | Shared classic selection framework. |
| Rust-only named save/load browser | no C++ equivalent | **Fail-fast** | Remove it; use the classic ten-slot flow. |

## Reusable modal infrastructure required

Full recursive parity requires classic implementations of these C++ dialog
families before individual call sites can be considered complete:

- message/error/info;
- Yes/No and multi-button confirmation;
- text/password input;
- timed confirmation;
- wait/progress with cancel;
- scrolling text/info;
- context and nested context menus;
- file/image/player/definition selection.

Unsupported entry points must keep logging and returning an error until their
classic implementation lands. Reintroducing a generic pane is not completion.

## Editor-only GUI, tracked separately

- `C4Console` native menu bar, script entry, player/viewport/help menus.
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
