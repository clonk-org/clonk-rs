//! `main.rs` — GameApp's state structs, lobby/menu/search state and boundary enums.
//!
//! A contiguous slice moved verbatim from the crate root; it stays part of
//! the same binary crate, re-exported from `main.rs` so every path resolves.

use super::*;
use crate::update_check::PendingUpdateCheck;
use crate::update_download::PendingUpdateDownload;

#[derive(Debug, Default)]
pub(crate) struct NetworkSavegameRecreationProgress {
    pub(crate) cursor: usize,
    /// `ChangeToLocal` tears down the manager, but Game.Control keeps the
    /// original local client identity while InitPlayers resumes.
    pub(crate) local_client_id: i32,
    /// The native outer RecreatePlayers packet loop freezes the client and
    /// name while its inner player loop may block on a resource.
    pub(crate) active_client: Option<(i32, String)>,
    /// `C4PlayerList::FileInUse` spans the whole resumable recreation walk,
    /// not just the one source processed before a resource wait.
    pub(crate) filename_ledger: clonk_engine::RuntimeJoinPlayerFilenameLedger,
}

/// One open component editor: which component, the text being edited, and the
/// host that will hold the accepted bytes.
///
/// C++ needs no such pairing — `C4ComponentHost` *is* the dialog, holding both
/// the bytes and the window. The port splits them because the editing surface
/// is invented and the commit rules are ported, and the two should not be one
/// type.
pub(crate) struct DeveloperComponentEdit {
    pub(crate) component: clonk_engine::developer_components::EditableComponent,
    pub(crate) text: crate::developer_component_editor::ComponentEditorText,
    pub(crate) host: clonk_engine::developer_components::ComponentHost,
}

/// The IRC transport and every chat surface that talks to it.
///
/// C++ keeps `C4Network2IRC` process-global and independent of the dialogs
/// that display it, so the transport, the two chat views that share it, and
/// the input state they hand back and forth are one lifecycle: closing a
/// view must not disturb the connection, and changing startup screens must
/// not tear it down. Flat on `GameApp` that relationship is invisible.
#[derive(Default)]
pub(crate) struct ChatState {
    /// Process-global C4Network2IRC analogue. Native retains the IRC client
    /// independently of the startup network dialog, so changing startup
    /// screens must not tear down a live connection.
    pub(crate) client: Option<clonk_network::IrcClientHandle>,
    pub(crate) server: String,
    /// Whether the singleton-style C4ChatDlg analogue is currently shown.
    /// The controller and transport remain process-global when this UI closes.
    pub(crate) external_dialog_visible: bool,
    /// Standalone C4ChatDlg owns a distinct C4ChatControl from the startup
    /// NetworkGame sheet. Closing the window destroys this UI-local state
    /// while the process-global IRC transport survives.
    pub(crate) external_dialog: Option<clonk_frontend::startup_netdlg::NetDlgController>,
    /// Distinguishes a resolver/TCP failure before the first Connected event
    /// (classic modal failure) from an established-session disconnect (status
    /// transcript line only).
    pub(crate) initial_connect_pending: bool,
    /// Shared double-click latch for the embedded and standalone chat views.
    pub(crate) dialog_last_click: Option<(GuiPoint, Instant)>,
    /// Left gesture retained by the standalone z=0 chat dialog.
    pub(crate) external_pointer_capture: bool,
    /// Process-global C4ChatInputDialog projection for ordinary game chat.
    pub(crate) running: Option<RunningChatState>,
    /// A paste may close its chat owner on key-down; retain the physical V
    /// until release so the replacement screen cannot receive an orphaned up.
    pub(crate) paste_consumed_keys: HashSet<VirtualKeyCode>,
    /// C4GUI::Edit retains an equal start/end selection as its hidden drag
    /// anchor even though the painted selection is empty.
    pub(crate) lobby_drag_anchor: Option<usize>,
}

/// The app's side of saving a game: where it writes, what it still owes,
/// and the language the description is written in.
///
/// The engine owns the deterministic save primitives; everything here is
/// I/O and presentation around them, and it moves as a unit — a save picks
/// a path, queues a thumbnail the renderer has not produced yet, and hands
/// the write to a background worker whose timings the next save reports.
#[derive(Default)]
pub(crate) struct SaveState {
    /// Byte-exact process-global resource table used by LoadResStr. Unlike a
    /// file reload at save time, this changes only when Options reloads the
    /// application language.
    pub(crate) description_language_table: Option<RuntimeLanguageBytesTable>,
    /// First two bytes of the materialized Config.General.Language used for
    /// C4GameSave's Desc??.rtf entry name.
    pub(crate) description_language: Vec<u8>,
    pub(crate) pending_gpu_thumbnail_paths: VecDeque<PathBuf>,
    pub(crate) pending_native_thumbnails: VecDeque<PendingNativeSaveThumbnail>,
    pub(crate) background_worker:
        Option<save_worker::BackgroundSaveWorker<save_worker::BackgroundSaveCompletion>>,
    pub(crate) last_native_timings: Option<save_worker::NativeSaveTimings>,
    pub(crate) last_path: Option<PathBuf>,
    /// Joined infos selected by RestoreSavegameInfos for the distinct
    /// RecreatePlayers phase. They must never fall back into normal network
    /// JoinPlayer issuance while legacy runtime-player loading is deferred.
    pub(crate) deferred_network_recreation: Vec<(i32, i32)>,
    pub(crate) network_recreation_progress: Option<NetworkSavegameRecreationProgress>,
}

/// The scenario selector: what was discovered, what is shown, and the
/// gestures in flight over it.
///
/// Discovery runs off the UI thread and replaces the whole book at once,
/// so the catalog, the worker that rebuilds it, the reload latch that
/// decides whether the next show rediscovers, and the per-row `CanOpen`
/// cache are one generation moving together — a stale cache against a
/// fresh catalog is the bug this grouping makes hard to write.
#[derive(Default)]
pub(crate) struct ScenarioSelectorState {
    pub(crate) mode: ScenarioSelectorMode,
    pub(crate) catalog: HashMap<String, FrontendScenario>,
    /// Interactive scenario refreshes run outside the UI thread. The old
    /// menu tree remains live but hidden until this worker supplies the
    /// replacement vector, making completion one atomic book rebuild.
    pub(crate) discovery: Option<ScenarioSelectorDiscoveryState>,
    /// The constructor has already populated the first selector generation.
    /// Later shows must rediscover disk changes made while that generation was
    /// hidden, including savegames written by the running round.
    pub(crate) reload_on_next_show: bool,
    /// Cached `C4ScenarioListLoader::Entry::CanOpen` result for the current
    /// selector mode. Rows stay actionable; this controls label color only.
    pub(crate) entry_enabled: HashMap<String, bool>,
    /// Last scenario-list row click (index, time) for double-click detection
    /// (OnSelDblClick -> DoOK, C4StartupScenSelDlg.h:430).
    pub(crate) last_click: Option<(usize, Instant)>,
    /// Focus selected by RenameEdit completion during an outside pointer
    /// press. The same gesture may still activate its target, but C++
    /// Dialog::SetFocus cancels that target's focus transfer.
    pub(crate) rename_pointer_focus: Option<ScenselFocusSnapshot>,
    /// Last search-edit click for C4GUI::Edit's double-click word selection.
    pub(crate) search_last_click: Option<Instant>,
}

/// The audio device and everything whose lifetime is tied to it.
///
/// The mixer is one resource with several claimants: a frontend-to-scenario
/// fade still owns it while the scenario loads, runtime playback ownership
/// is distinct from the persisted `RXMusic` setting, and the once-per-entry
/// frontend guard exists so dialog navigation cannot restart a finished
/// track. Those three latches only make sense next to the context they
/// arbitrate over.
///
/// Voice chat is deliberately not here. It is presentation-only proximity
/// audio with its own state type, and its single open-mic gate is easier to
/// audit standing alone than folded into the mixer's lifetime.
#[derive(Default)]
pub(crate) struct SoundState {
    pub(crate) context: Option<SharedAudioContext>,
    #[cfg(test)]
    pub(crate) ui_log: Vec<String>,
    /// `C4Game::IsMusicEnabled`; runtime playback ownership remains distinct
    /// from persisted RXMusic while a game is running.
    pub(crate) runtime_music_enabled: bool,
    /// A frontend-to-scenario fade still owns the mixer. Scenario-load failure
    /// may return to Menu and wait for it; game teardown instead clears this at
    /// the following PreInit reconstruction before entering startup.
    pub(crate) resume_frontend_after_fade: bool,
    /// `C4Startup::DoStartup` calls PlayFrontendMusic exactly once per startup
    /// entry. Dialog navigation must not restart a non-looping track after it
    /// ends; returning from a game resets this guard.
    pub(crate) frontend_attempted_for_entry: bool,
}

/// Persisted configuration, the deferred writes that have not reached it
/// yet, and the process-level latches read from it.
///
/// C++ mutates one process-wide `Config` and saves it once in
/// `C4Application::Clear` (C4Application.cpp:351-367), so the deferred
/// writes and the values already on disk are two halves of one thing:
/// reading a setting has to consult both, and a round that earns mission
/// access must know what was already written to avoid rewriting it every
/// frame. The gamepad and display latches are process-local projections of
/// the same config, captured at the points native captures them.
pub(crate) struct ConfigState {
    /// Ordinary runtime config toggles, held until a clean shutdown the way
    /// C++ mutates its process-wide `Config` and saves once in
    /// `C4Application::Clear` (C4Application.cpp:351-367).
    pub(crate) deferred: crate::deferred_config::DeferredConfig,
    /// The one operating mode this run resolved to, from the launch override
    /// or the persisted `General.CompatProfile` key
    /// (`crate::settings::resolve_compat_profile`). Held as state rather than
    /// re-read per use so a host and the clients it admits cannot disagree
    /// about it mid-session.
    pub(crate) compat_profile: crate::settings::CompatProfile,
    /// Encoding of the process-global resource table. `LoadResStr` reads the
    /// same already-loaded table as the UTF-8 presentation helpers; retain
    /// its source charset so byte-returning call sites do not reopen it.
    pub(crate) language_charset: RuntimeHelpCharset,
    /// Process-local Config.General.MissionAccess shared across fresh games.
    pub(crate) mission_access: MissionAccessStore,
    /// The mission-access list already on disk, so a round that earns one
    /// writes once rather than on every frame that follows.
    pub(crate) persisted_mission_access: String,
    /// `Config.Graphics.ShowFolderMaps`, default-on like C4ConfigGraphics.
    pub(crate) show_folder_maps: bool,
    /// Process-local Config.Graphics.ShowCommands enable requests shared
    /// across fresh engines.
    pub(crate) show_commands_requests: ShowCommandsRequestStore,
    /// Current `Config.General.GamepadEnabled` value used by each new
    /// `C4Player::InitControl` analogue.
    pub(crate) gamepads_enabled: bool,
    /// Startup-time gamepad subsystem gate. Native does not create or later
    /// poll `C4GamePadControl` when this was false during application init.
    pub(crate) gamepad_input_enabled: bool,
    pub(crate) gamepad_gui_control: bool,
}

/// The startup screens: which one is showing, the controller behind it, and
/// the models it was built from.
///
/// `C4Startup` keeps one dialog alive at a time and fades between them, so
/// the view, the fade in flight, the back-target a scenario entry pushed,
/// and the per-screen controllers are one navigation state. The player and
/// crew vectors sit here too because they are what the selection screen was
/// built from: rebuilding the models without the files they came from is
/// the inconsistency this grouping makes visible.
pub(crate) struct StartupDialogState {
    pub(crate) player_dialog: Option<clonk_frontend::startup_plrsel::PlrSelController>,
    pub(crate) player_properties_dialog: Option<PendingStartupPlayerProperties>,
    /// Process-local C4Config state set after the first stock extraction
    /// attempt, independently of whether the config file can be saved.
    pub(crate) user_portraits_written: bool,
    /// Process-local C4Config row remembered when the selector closes.
    pub(crate) last_portrait_folder_index: Option<usize>,
    pub(crate) player_files: Vec<StartupPlayerFile>,
    pub(crate) player_models: Vec<clonk_frontend::startup_plrsel::PlrSelPlayer>,
    pub(crate) crew_files: Vec<StartupCrewFile>,
    pub(crate) crew_models: Vec<clonk_frontend::startup_plrsel::PlrSelCrew>,
    pub(crate) crew_player_index: Option<usize>,
    /// Inline `CallbackRenameEdit` projected over the active crew row.
    pub(crate) crew_rename: Option<StartupCrewRenameState>,
    pub(crate) options_dialog: Option<clonk_frontend::startup_options_dlg::OptionsDlgState>,
    pub(crate) options_advanced_dialog: Option<PendingOptionsAdvancedDialog>,
    pub(crate) about_dialog: Option<clonk_frontend::startup_about_dlg::AboutDlgState>,
    pub(crate) view: StartupView,
    /// C4Startup::SwitchDialog's paired FadeOut/FadeIn. The outgoing pixels
    /// are frozen because opening the replacement mutates or destroys several
    /// startup controllers before the next presentation.
    pub(crate) dialog_fade: Option<StartupDialogFade>,
    /// The one retained dialog that native `SDID_Back` may reuse. A dialog
    /// rebuilt by `DoStartup` after a round has no such history, so Back from
    /// that fresh selector goes to Main instead of inventing a NetDlg.
    pub(crate) scenario_back_dialog: Option<StartupDialog>,
    pub(crate) view_flags: StartupViewFlags,
}

/// The dialogs a running game can put on screen, and the stack that orders
/// them.
///
/// `C4GUI::Screen` keeps one absolute z-order across every shown dialog, so
/// the scoreboard, the client list, the statistics chart and the message
/// dialogs are not independent toggles: which one is topmost decides which
/// one a key or a click reaches, and each keeps its own consumed-key and
/// pointer-capture latches for exactly that reason. Held flat, the stack sat
/// several hundred lines from the dialogs it orders.
#[derive(Default)]
pub(crate) struct RuntimeDialogState {
    /// Singleton runtime `C4Network2ClientListDlg`, toggled by bare F4.
    pub(crate) client_list: Option<clonk_frontend::runtime_client_list::RuntimeClientListDialog>,
    /// Keys owned by the modal lobby `C4Network2ClientDlg` until release,
    /// including the Escape press that closes it.
    pub(crate) client_list_consumed_keys: HashSet<VirtualKeyCode>,
    /// Both this dialog and game-over use C4GUI's default z=0. Preserve which
    /// one was shown most recently so equal-z rendering and input stay in the
    /// native insertion order.
    pub(crate) client_list_above_game_over: bool,
    /// Bottom-to-top order of the running Screen's default-z dialogs.
    pub(crate) default_order: Vec<RuntimeDefaultDialog>,
    /// Runtime-only C4Scoreboard::pDlg lifecycle. The engine owns the saved
    /// cells/refcount; this flag changes only at DoDlgShow/game-start/Tab and
    /// the explicit game-over/Clear close sites.
    pub(crate) scoreboard: Option<ScoreboardPresentationRequest>,
    pub(crate) scoreboard_initial_reconcile_pending: bool,
    pub(crate) scoreboard_close_pointer_capture: bool,
    pub(crate) scoreboard_runtime: ScoreboardDialogRuntime,
    /// Bottom-to-top native Screen child order for the running shared-dialog
    /// layers whose z/activation interactions cross controller boundaries.
    pub(crate) stack: Vec<RunningDialogStackEntry>,
    /// App-owned classic dialogs, in C4GUI z-order. Pointer hit-testing starts
    /// at the top; every entry is rendered bottom-to-top without a scrim.
    pub(crate) messages: Vec<PendingMessageDialog>,
    /// Process-global C4ChartDialog singleton and its stronger Escape latch.
    pub(crate) chart: Option<clonk_frontend::network_chart::NetworkChartDialog>,
    pub(crate) chart_consumed_keys: HashSet<VirtualKeyCode>,
    pub(crate) chart_pointer_capture: bool,
    /// `ActivateDialog`/new-dialog insertion can move the z=0 chart after
    /// already shown z=+1/+2 dialogs in the Screen's absolute list.
    pub(crate) chart_elevated: bool,
    /// `C4GraphicsSystem::ShowHelp`; reset by GraphicsSystem::Default for a
    /// new game and toggled by each in-scope F1 down edge.
    pub(crate) help_visible: bool,
    /// C4's process-global resource string table is fixed at application
    /// startup. Cache the resolved help columns likewise; errors are retained
    /// so no later frame can silently switch languages.
    pub(crate) help_text_cache: OnceLock<std::result::Result<RuntimeHelpColumns, String>>,
    /// Wooden menu title currently moving its owning dialog.
    pub(crate) menu_title_drag: Option<MenuTitleDrag>,
}

/// Live keyboard and pointer state: what the platform last told us, and
/// what `C4MouseControl` made of it.
///
/// The two halves are one chain. A platform move updates the window
/// position, which the running pointer follows, which resolves against a
/// viewport into the retained mouse — and ownership (`fMouseOwned` for the
/// GUI versus the world) decides which of those the next click reaches.
/// The click-synthesis latch lives here too because winit reports no OS
/// click count, so the port has to derive one from the same timeline.
pub(crate) struct InputState {
    /// Engine-routed physical keys currently held by the window input
    /// backend. Winit's repeated `Pressed` events must carry C++'s
    /// `fRepeated` semantics into `LocalControlKey` rather than looking like
    /// deliberate double presses.
    pub(crate) pressed_engine_keys: HashSet<VirtualKeyCode>,
    pub(crate) modifiers: ModifiersState,
    pub(crate) gamepads: GamepadManager,
    /// Last mouse-only logical position. Touch input intentionally does not
    /// materialize C4GUI's themed mouse pointer.
    pub(crate) window_pointer: Option<GuiPoint>,
    /// Platform client-area membership is independent of focus: focus loss
    /// restores the OS cursor without forgetting where a stationary pointer
    /// will be when focus returns.
    pub(crate) pointer_inside_window: bool,
    pub(crate) running_pointer: Option<GuiPoint>,
    /// Exact C4GUI::CMouse ownership for the most recent running mouse move.
    pub(crate) gui_mouse_owned: bool,
    /// Exact C4MouseControl::fMouseOwned state. This is deliberately separate
    /// from GUI ownership: MouseControl::Init resets this bit to true without
    /// clearing C4GUI's bit, so both cursors can transiently be active.
    pub(crate) world_mouse_owned: bool,
    /// Physical primary-button state (`CMouse::LDown`), independent of any
    /// control that installed itself as `pDragElement`.
    pub(crate) primary_left_down: bool,
    /// SDL/X11 classify the second application-wide left press as LeftDouble
    /// before GUI hit-testing, regardless of which control saw the first.
    pub(crate) last_left_press: Option<Instant>,
    /// Raw window position used by C4GUI-style viewport/menu hit-testing.
    /// Gameplay keeps a separate pointer because C4MouseControl clamps raw
    /// positions into its assigned viewport (C4MouseControl.cpp:1216-1227).
    pub(crate) ingame_gui_pointer: Option<GuiPoint>,
    pub(crate) ingame_pointer: Option<ViewportPointer>,
    /// `C4MouseControl::Help`, activated locally by the viewport Help button.
    /// This is deliberately independent from the F1 `ShowHelp` overlay.
    pub(crate) ingame_mouse_help: bool,
    /// `C4MouseControl::InitCentered`: the first viewport move after `Init`
    /// is evaluated at the viewport center, regardless of the platform point.
    pub(crate) ingame_mouse_init_centered: bool,
    /// C4MouseControl::VpX/VpY and its physical viewport identity. These are
    /// retained even away from an edge so the native Tick5 synthetic Move
    /// can reevaluate regions and layout changes without a new OS event.
    pub(crate) ingame_viewport_mouse: Option<RetainedViewportMouse>,
    /// Retained C4MouseControl::Scrolling direction. Move applies it once;
    /// every subsequently executed game tick applies it again until the
    /// pointer leaves the exact clamped viewport border.
    pub(crate) ingame_edge_scroll: Option<ActiveViewportEdgeScroll>,
    /// Presentation-only C4MouseControl caption timing and placement.
    pub(crate) ingame_mouse_caption: IngameMouseCaptionState,
    /// `C4MouseControl::TargetObject`, retained by the last Move/Tick5 refill.
    /// Button events consume this identity without repeating hit-testing.
    pub(crate) ingame_mouse_target: Option<ObjectId>,
}

/// The app's side of who is playing: the local profile, and the rosters a
/// host reassembles around it.
///
/// `clonk-engine` owns the deterministic players. What is left here is the
/// orchestration that decides which ones exist: the local owner and its
/// profile, the infos and roster items a restart must restore rather than
/// rejoin fresh, the host's own player ids and their resource-alternate
/// colours, and the team assignment those joins resolve against. Restart
/// restoration in particular is only correct when read together — an info
/// restored without its roster item rejoins as a new player.
#[derive(Default)]
pub(crate) struct PlayerState {
    pub(crate) local_owner: i32,
    pub(crate) local_name: String,
    pub(crate) selected_file: Option<PlayerFile>,
    /// Native restart handoff captured at full game initialization and kept
    /// across the next same-scenario lobby only.
    pub(crate) restart_restore_infos: RestartRestoreInfos,
    /// PlayerListItem runs its restore hook only on construction, not on each
    /// later row update. Track the items already constructed in this lobby.
    pub(crate) restart_restore_roster_items: HashSet<(i32, i32)>,
    /// Assigned PlayerInfo identities that still belong to this host process.
    /// Resource IDs are global and may also be referenced by a remote row, so
    /// they cannot by themselves prove ownership of the local-only color.
    pub(crate) host_local_info_ids: HashSet<i32>,
    /// Process-local `C4PlayerInfo::dwAlternateColor` values for players
    /// loaded by this host. The synchronized row intentionally omits this
    /// field, so resource identity carries it across authoritative echoes and
    /// later conflict-resolution passes.
    pub(crate) host_local_alternate_colors: HashMap<i32, u32>,
    pub(crate) team_assignment: Option<NetworkTeamAssignmentState>,
    /// Frozen Application.ResStrTable template used by GenerateDefaultTeams.
    pub(crate) generated_team_name_template: LegacyCString,
}

/// Control recording and playback: whether to record, what to record
/// against, and the session doing it.
///
/// `C4GameControl` treats a record as a save plus a control stream, which
/// is why the seed sits here beside the session: the template and the live
/// session are both written against the same scenario and parameters, and
/// `fRecordNeeded` is armed by the console but only cleared once the queued
/// `CID_Synchronize` actually starts the record. Playback is the same state
/// read backwards, so it belongs with them rather than beside the engine it
/// feeds.
///
/// Named `records` rather than `recording`: the app field must not share
/// a name with the `recording` session inside it, or a site that meant the
/// session resolves to the whole state and only fails later, on a type.
#[derive(Default)]
pub(crate) struct RecordingState {
    pub(crate) enabled: bool,
    pub(crate) directory: Option<PathBuf>,
    /// Scenario and parameter inputs shared by developer saves and
    /// non-initial records. This survives consuming `recording_template`.
    pub(crate) live_save_seed: Option<RuntimeRecordingSeed>,
    pub(crate) template: Option<RecordingTemplate>,
    pub(crate) session: Option<RecordingSession>,
    /// `C4GameControl::fRecordNeeded`: set as soon as the developer console
    /// requests a runtime record and cleared only when its queued
    /// `CID_Synchronize` starts the record (or submission fails).
    pub(crate) runtime_requested: bool,
    pub(crate) playback: Option<ControlRecordPlayback>,
}

pub(crate) struct GameApp {
    pub(crate) engine: Engine,
    pub(crate) graphics: GraphicsSystem,
    pub(crate) sky: Option<SkyRenderState>,
    /// Landscape texture surfaces + material render metadata (re-applied on
    /// every GraphicsSystem rebuild, like the sky).
    pub(crate) material_texture_images: Arc<HashMap<String, MaterialTextureSurface>>,
    pub(crate) material_render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    /// System.c4g global script sources, loaded once at boot for every
    /// fresh game engine (the C++ `Game.ScriptEngine` scripts).
    pub(crate) system_scripts: Vec<(String, String)>,
    /// System.c4g Names.txt, used when fresh crew files have no definition-
    /// specific ClonkNames list (C4Game::InitScriptEngine).
    pub(crate) standard_names: Option<String>,
    /// Process-global Application.ResStrTable entries used by
    /// GetNeededMatStr. Rebuilt after an Options language selection and
    /// reinstalled on every fresh Engine.
    pub(crate) needed_material_need: String,
    pub(crate) needed_material_none: String,
    /// Process-global localized `IDS_OBJ_NODIG`, reinstalled on every fresh
    /// engine and refreshed immediately after an Options language change.
    pub(crate) object_no_dig: String,
    /// Localized ConstructionCheck feedback bundle, managed exactly like
    /// `object_no_dig`.
    pub(crate) construction_check_feedback: [String; 5],
    /// Game.Rank names frozen by the latest C4Game::PreInit analogue. Startup
    /// Options may reload the process language table afterward, but a game
    /// started from that same startup session retains these names.
    pub(crate) default_rank_names: Option<Vec<String>>,
    /// IDS_GAME_DEFRANKS from the currently loaded process language table.
    /// The next return-to-startup PreInit promotes this to Game.Rank.
    pub(crate) loaded_default_rank_names: Option<Vec<String>>,
    /// Process-global `Application.ResStrTable` projection shared by startup,
    /// lobby, console, and runtime presentation. C++ replaces this table only
    /// after an Options language selection; retaining it also prevents
    /// per-frame network-browser/console lookup from reopening System.c4g.
    pub(crate) startup_tooltip_resources: HashMap<String, String>,
    /// Process-global C4Group maker captured from `Config.General.Name` once
    /// during application initialization, like `C4Group_SetMaker`.
    pub(crate) process_group_maker: LegacyCString,
    /// Persisted config, its deferred writes, and the latches over it.
    pub(crate) config: ConfigState,
    /// Where a save writes, what it still owes, and in which language.
    pub(crate) saves: SaveState,
    /// Process-local `Config.General.AllowScriptingInReplays`; native reads
    /// this from its already-loaded configuration while replay controls run.
    pub(crate) allow_scripting_in_replays: bool,
    pub(crate) input: InputDispatcher,
    pub(crate) bindings: KeyboardBindings,
    pub(crate) gamepad_bindings: GamepadBindings,
    pub(crate) local_controls: LocalControlRegistry,
    /// `fRepeated` for the physical key event currently being routed.
    ///
    /// `C4Game::DoKeyboardInput` derives it from `PressedKeys` as its very
    /// first statement — before keyboard scope is computed and before any
    /// dialog can claim the event (`C4Game.cpp:2143-2155`) — then carries it
    /// down as a parameter. `GameApp::handle_key` resolves it once at the same
    /// point so every early-returning handler leaves the same latch behind.
    pub(crate) engine_key_repeated: bool,
    /// Whether the physical-key route consumed this event before winit's
    /// paired `KeyEvent::text` is considered. Push-to-talk keys use this to
    /// avoid also typing into a focused game-shell edit control.
    pub(crate) key_event_suppresses_text: bool,
    /// Raw Tab state is tracked before modifier/dialog scope lookup because a
    /// held key can cross into or out of a PRIO_PlrControl binding.
    pub(crate) scoreboard_tab_raw_pressed: bool,
    pub(crate) pending_screenshots: VecDeque<ScreenshotRequest>,
    pub(crate) retained_gpu_presentation_active: bool,
    /// While scale-native text is captured, split the retained command stream
    /// at the same painter-order boundaries as `NativePresentationPlan`.
    pub(crate) retained_gpu_ordered_capture_active: bool,
    /// Reused command-only target for scale-native physical text layers.
    pub(crate) retained_native_capture_surface: Option<Surface>,
    pub(crate) pending_options_display_requests: VecDeque<OptionsDisplayRequest>,
    #[cfg(test)]
    pub(crate) gamepad_poll_count: usize,
    #[cfg(test)]
    pub(crate) sec1_timer_call_count: usize,
    pub(crate) snapshot: SimulationSnapshot,
    pub(crate) focus_id: Option<ObjectId>,
    pub(crate) focus_snapshot: Option<clonk_engine::ObjectSnapshot>,
    pub(crate) frame_text: String,
    pub(crate) status_text: String,
    pub(crate) startup_restart_diagnostics: StartupRestartDiagnostics,
    pub(crate) energy_fraction: f32,
    pub(crate) scenario_label: String,
    pub(crate) fallback_ground: i32,
    pub(crate) menu_state: MenuState,
    pub(crate) main_menu_state: MainMenuState,
    /// One process-level C4GUI::CMouse tooltip clock shared by every startup
    /// dialog. Target lookup remains view-specific, but input ownership and
    /// the 500ms stillness delay are global.
    pub(crate) startup_tooltip: ClassicTooltipTracker,
    /// Which startup screen is showing and what built it.
    pub(crate) startup: StartupDialogState,
    pub(crate) startup_network_dialog: Option<clonk_frontend::startup_netdlg::NetDlgController>,
    /// IRC and every chat surface, which share one transport lifetime.
    pub(crate) chat: ChatState,
    /// `Application.launchEditor`: set by `SwitchToEditor`, consumed by
    /// `~C4Application` after subsystem cleanup (C4Application.cpp:58-74).
    pub(crate) pending_editor_launch: Option<PathBuf>,
    pub(crate) startup_game_search: Option<clonk_network::StartupGameSearch>,
    #[cfg(test)]
    pub(crate) startup_game_search_test_events: VecDeque<clonk_network::StartupGameSearchEvent>,
    /// C4StartupNetDlg::tLastRefresh. OnShown seeds the one-second guard,
    /// and accepted Reload/F5 requests advance it before restarting search.
    pub(crate) startup_network_last_refresh: Option<Instant>,
    /// C4StartupNetListEntry::iTimeout for the masterserver row. Unlike
    /// tLastRefresh, this response-relative deadline must not throttle F5.
    pub(crate) startup_masterserver_next_query_at: Option<Instant>,
    /// C4StartupNetListEntry::iRequestTimeout for the masterserver row. The
    /// worker owns the transport deadline, but native also bounds how long the
    /// row may display IDS_NET_INFOQUERY, so a query whose outcome never
    /// reaches the dialog still resolves (src/C4StartupNetDlg.cpp:182,216-223).
    pub(crate) startup_masterserver_request_timeout_at: Option<Instant>,
    /// Reject pre-refresh events until the worker acknowledges the new
    /// generation with Cleared, so deleted rows cannot flash back into view.
    pub(crate) startup_network_refresh_waiting_for_clear: bool,
    /// `C4StartupNetDlg::fIgnoreUpdate`: declining one league-server redirect
    /// suppresses further prompts for the lifetime of this dialog instance.
    pub(crate) startup_network_ignore_redirect: bool,
    /// Complete references retained in the same order as the visible game
    /// list. The frontend row projects only a display address.
    pub(crate) startup_game_references: Vec<clonk_network::NetworkGameReference>,
    /// Per-host reference requests created from LAN discovery datagrams.
    pub(crate) startup_discovery_reference_queries: Vec<StartupDiscoveryReferenceQuery>,
    /// User-entered reference requests remain visible until they resolve,
    /// fail, or are replaced by their returned reference rows.
    pub(crate) startup_direct_reference_queries: Vec<StartupDirectReferenceQuery>,
    pub(crate) next_startup_direct_reference_query_id: u64,
    pub(crate) network_game_advertiser: Option<clonk_network::NetworkGameAdvertiser>,
    /// Last validated exact host reference. This state advances independently
    /// of optional listener I/O and is retained as the next InitLocal rebuild
    /// template when advertising could not bind.
    pub(crate) advertised_game_reference: Option<clonk_network::HostGameReference>,
    /// Process-local `C4Startup::eLastDlgID`. The network lobby and staged
    /// loader are game states rather than startup dialogs, so they must not
    /// displace the dialog reopened after the round ends.
    pub(crate) last_startup_dialog: StartupDialog,
    pub(crate) scenario_game_options: GameOptionButtons,
    pub(crate) object_menu: Option<ObjectMenuState>,
    pub(crate) ingame_menu: PlayerIngameMenus,
    /// Cached Graphics.c4g sheets for the in-game menu renderer.
    pub(crate) ingame_menu_gfx: Option<IngameMenuGraphics>,
    /// `C4Player::BigIcon` equivalents keyed by stable C4PlayerInfo ID. The
    /// renderer projects these onto the current runtime player numbers.
    pub(crate) runtime_player_big_icons: HashMap<i32, ImageData>,
    /// Player-info sources already checked without finding a usable BigIcon.
    pub(crate) runtime_player_big_icon_misses: HashSet<i32>,
    /// Per-viewport-owner async C4Menu::TimeOnSelection presentation state.
    /// This is deliberately outside the deterministic engine menu state
    /// (C4Menu.cpp:804-821).
    pub(crate) script_menu_presentations: BTreeMap<i32, ScriptMenuPresentationState>,
    /// Last output rectangle for each owning viewport. C4Viewport raises
    /// `ResetMenuPositions` on split-screen relayouts as well as window
    /// resizes, so menu placement cannot key only off the OS resize event.
    pub(crate) menu_viewport_rects: BTreeMap<i32, Rect>,
    /// `Config.Graphics` display toggles loaded at process startup and driven
    /// by the Display submenu (C4MainMenu.cpp:855-884).
    pub(crate) display_flags: DisplayFlags,
    /// `Config.General.UseWhiteLobbyChat`, which is intentionally distinct
    /// from the in-game white-chat display toggle.
    pub(crate) white_lobby_chat: bool,
    /// Prefix GUI log lines with C++'s markup-colored wall-clock timestamp.
    pub(crate) show_log_timestamps: bool,
    /// Process-local `Config.Graphics.SmokeLevel`; object bubbles consult it
    /// only outside network/recording sync mode, while particles always do.
    pub(crate) graphics_smoke_level: i32,
    /// `C4Player::MouseControl` analogue: gates in-game mouse gameplay
    /// input (C4MainMenu.cpp:847-849).
    pub(crate) mouse_control: bool,
    /// False when `[Head] DisableMouse=1`; C++ then neither assigns mouse
    /// control nor offers its Options entry (C4Player.cpp:1907-1912;
    /// C4MainMenu.cpp:563-571).
    pub(crate) mouse_control_allowed: bool,
    pub(crate) mode: AppMode,
    /// The scenario selector's own state.
    pub(crate) scensel: ScenarioSelectorState,
    pub(crate) active_scenario: Option<FrontendScenario>,
    /// Effective definition vector from the active game. C++ backs this up
    /// across Restart/Next Mission and restores it as FixedDefinitions.
    pub(crate) active_definition_load: Option<ScenarioDefinitionLoad>,
    /// Byte-exact `Game.DefinitionFilenames` projection used only by
    /// C4GameSave::WriteDescDefinitions. The String-based load vector cannot
    /// retain native Unix path bytes that are not valid UTF-8.
    pub(crate) active_description_definition_modules: Vec<Vec<u8>>,
    /// C4GraphicsResource's game-local HUD, cursor and palette selection.
    /// `None` means the process-startup Graphics.c4g bundle is active.
    pub(crate) active_game_graphics: Option<GameGraphicsResources>,
    /// The audio device and its music lifetime.
    pub(crate) sound: SoundState,
    /// Presentation-only proximity voice state; never serialized or passed to
    /// the deterministic engine.
    pub(crate) voice_chat: crate::voice_chat::VoiceChatState,
    #[cfg(test)]
    pub(crate) league_surrender_pre_abort_results: Option<(
        clonk_engine::RoundResultsState,
        clonk_engine::RoundResultsState,
        bool,
    )>,
    pub(crate) assets: Arc<FrontendAssets>,
    /// Per-resource failures from resolving the active scenario's C4GUI
    /// sheet/font set. Empty means the active (or startup) bundle resolved
    /// exactly; any entry keeps the typed bootstrap boundary failing.
    pub(crate) active_global_gui_failures: HashMap<&'static str, String>,
    /// Scale-native CStdFont atlas used after the presenter's bilinear base
    /// pass for startup screens and in-game messages. C++ rebuilds its
    /// fonts with Application.GetScale()
    /// (C4Fonts.cpp:158-173).
    pub(crate) native_startup_fonts: Option<Arc<clonk_frontend::clonk_fonts::NativeClonkFontSet>>,
    /// Ordered logical chrome/native-text batches prepared during the current
    /// logical render and consumed immediately after FramePresenter upscales
    /// the base. Keeping later chrome in separate batches preserves C4GUI
    /// z-order instead of replaying every glyph over the finished frame.
    pub(crate) pending_native_presentation: Option<NativePresentationPlan>,
    /// Exact C4LoaderScreen selected for the currently active startup or
    /// scenario load. A missing screen is paired with `loader_error` and is
    /// always a logged typed boundary, never a generic pane.
    pub(crate) loader_screen: Option<LoaderScreen>,
    /// Loader percentage mirrored to the platform taskbar
    /// (`C4Game.cpp:4094-4106`; `StdWindow.cpp:183-196`). The backend is
    /// injected because C++'s SDL and X11 windows implement it as no-ops.
    pub(crate) taskbar_progress: clonk_platform::taskbar_progress::LoaderTaskbarProgress<
        Box<dyn clonk_platform::taskbar_progress::TaskbarProgressSink>,
    >,
    /// Why no `loader_screen` is installed, when one was attempted.
    pub(crate) loader_error: Option<LoaderScreenFailure>,
    pub(crate) loader_render_config: Option<LoaderRenderConfig>,
    pub(crate) loader_render_error: Option<String>,
    pub(crate) loader_gamma: Option<clonk_graphics::GammaRamp>,
    pub(crate) app_paths: Option<AppPaths>,
    /// Process-local compatibility arguments applied after configuration is
    /// loaded. They must never be written back to the selected config file.
    pub(crate) classic_command_line: ClassicCommandLine,
    /// A converted command-line stream still owns the initial OpenGame
    /// attempt. Clear this after successful activation so later rounds do not
    /// inherit the native no-startup failure policy.
    pub(crate) classic_record_stream_activation_pending: bool,
    /// ParseCommandLine snapshots the config/`.c4d` definition vector once
    /// for the next game init. Later startup rounds begin from an empty Game
    /// and the unchecked selector appends only Objects.c4d.
    pub(crate) initial_definition_seed: Option<Vec<String>>,
    /// Persistent developer-window policy selected by `/console`. Unlike
    /// per-round classic arguments, `/open` must not reset this.
    pub(crate) console_mode: bool,
    /// Dedicated-server policy selected by `--headless`: no window, no render
    /// device, no sound. C++ makes this a build (`USE_CONSOLE`,
    /// CMakeLists.txt:178) whose `DDrawInit` compiles the OpenGL arm out
    /// entirely (StdDDraw2.cpp:1305-1310), so it also implies
    /// `lpDDraw->GetEngine() == GFXENGN_NOGFX`.
    ///
    /// Deliberately *not* `console_mode`: that grants developer-console
    /// authority — it is the `Console.Active` argument behind
    /// `ScriptControlPolicy::live`, i.e. "execute remote console-scope script
    /// from any client" — which a server exposed to the internet must not
    /// inherit. C++ reads the two separately too, and either alone makes the
    /// lobby a console lobby (C4Network2.cpp:463).
    pub(crate) headless: bool,
    /// Control ticks this client sent that the host's async deadline gave up
    /// on, so the input never executed anywhere
    /// (`force_expired_async_control`, mirroring `PackCompleteCtrl`,
    /// C4GameControlNetwork.cpp:741-784). A diagnostic count only — it is read
    /// back by the network diagnostics and never by simulation, because the
    /// host alone decides the timeout and every client executes the one
    /// aggregate it broadcasts.
    pub(crate) discarded_control_ticks: u32,
    /// The last discarded tick already reported to the player, so a peer that
    /// loses several ticks in a burst is described once per tick rather than
    /// once per redelivery.
    pub(crate) last_reported_discarded_control_tick: Option<i32>,
    /// A console command has put `Application.UseStartupDialog` back, so this
    /// session has a startup generation to return to even though it was
    /// launched without one. `/open` and `/close` both set it
    /// (C4Application.cpp:598-612,617-624), which is what lets a dedicated
    /// server park for the next command instead of ending its process.
    pub(crate) console_restored_startup_dialog: bool,
    pub(crate) developer_console: DeveloperConsole,
    pub(crate) developer_console_edit_mode: ConsoleEditMode,
    /// `C4EditCursor::Selection`. Shared by the viewport edit cursor, the
    /// property panel and the object tree, so a write from one is visible
    /// to the others (`C4EditCursor.h:39`).
    pub(crate) developer_selection: clonk_engine::developer_selection::DeveloperSelection,
    /// `C4Console::ToolsDlg` — the retained tool, grade, IFT, material and
    /// texture the Draw-mode gestures read, plus their own `Hold`/anchor.
    /// Away from Win32 and GTK this state *is* the tools dialog: `Open`
    /// creates no window at all on the reference build (`C4ToolsDlg.cpp:262`).
    pub(crate) developer_tools: clonk_engine::developer_tools::DeveloperTools,
    /// Which Tools-page selector is showing its list.
    ///
    /// Presentation state, so it lives here rather than in the engine's
    /// `DeveloperTools`: C++'s combo owns its own dropped-down state and the
    /// dialog reads it back, and nothing about it reaches the simulation.
    pub(crate) developer_tools_open_combo: Option<crate::developer_toolbox_view::ToolsCombo>,
    /// The projection each console viewport window was last drawn with,
    /// keyed by physical identity. `GraphicsSystem::active_viewports` holds
    /// the *fullscreen* layout, which console mode never renders, so a
    /// detached window's pointer routing has no other source of its own
    /// `ViewX`/`ViewY` (`C4Viewport.cpp:1146`).
    /// The object list's retained scroll position, and the selection it was
    /// last moved for. `C4ObjectListDlg::Update` rebuilds the model on every
    /// object change and only moves the view when the *cursor* changes
    /// (`C4ObjectListDlg.cpp:599-646,747-780`), so the reveal has to fire on a
    /// selection change rather than on every frame — otherwise scrolling away
    /// from the selection would be impossible.
    pub(crate) developer_object_list_scroll: crate::developer_object_list_view::ObjectListScroll,
    /// Which developer pane's scroll thumb is being dragged.
    ///
    /// One field for both panes because they live in different windows: a
    /// drag can only ever be in one of them, and holding it here is what lets
    /// the release find it wherever the pointer ended up.
    pub(crate) developer_pane_scroll_drag: Option<DeveloperPane>,
    /// The object list's keyboard cursor.
    ///
    /// `GtkTreeView` keeps a cursor separate from the selection: Ctrl+arrows
    /// move it without selecting, and Ctrl+Space then selects what it is on.
    /// A cursor whose row is no longer drawn is not a position, so navigation
    /// starts over from the top rather than guessing.
    pub(crate) developer_object_list_cursor: Option<clonk_engine::ObjectId>,
    /// Where a Shift-extended range is anchored.
    pub(crate) developer_object_list_anchor: Option<clonk_engine::ObjectId>,
    /// Which containers the user has opened in the object tree
    /// (`C4ObjectListDlg.cpp:726-787`).
    pub(crate) developer_object_tree_expansion:
        crate::developer_object_list_view::ObjectTreeExpansion,
    pub(crate) developer_object_list_revealed: Option<clonk_engine::ObjectId>,
    /// What is typed into the property page's script entry.
    ///
    /// `IDC_COMBOINPUT`'s own text: `UpdateInputCtrl` reads it before
    /// rebuilding the completion list and writes it back afterwards
    /// (`C4PropertyDlg.cpp:296-306,372-374`), so the rebuild `Update` performs
    /// on Tick35 and on every selection change cannot eat a half-typed call.
    /// Holding it here is that preservation: nothing else writes it.
    pub(crate) developer_property_script_input: String,
    /// The property pane's retained first visible line
    /// (`C4PropertyDlg.cpp:257-262`).
    pub(crate) developer_property_scroll: crate::developer_toolbox_view::LineScroll,
    /// The viewport whose scroll thumb is being dragged, and on which axis.
    ///
    /// Per physical viewport: two detached windows can be dragged one after
    /// the other without the first one's capture answering for the second.
    pub(crate) console_viewport_scroll_drag:
        Option<(u64, clonk_engine::developer_viewport::ScrollAxis)>,
    pub(crate) console_viewport_projections:
        std::collections::HashMap<u64, clonk_frontend::ActiveViewportProjection>,
    /// The last pointer position in world coordinates, so a held drag can
    /// send `MoveSelection` the *delta* C++ computes from the previous
    /// message's coordinates (`C4EditCursor.cpp:131-137`).
    pub(crate) edit_cursor_last_world: Option<(i32, i32)>,
    /// `C4EditCursor::DropTarget` — the container a Ctrl-drag would put the
    /// selection into, recomputed on every motion (`UpdateDropTarget`).
    pub(crate) edit_cursor_drop_target: Option<clonk_engine::ObjectId>,
    /// The engine frame the held-move control was last issued for.
    /// `C4Console::Execute` runs `EditCursor.Execute()` once per
    /// application tick; the port's event loop wakes far more often than
    /// that, and emitting per wake would flood the control queue.
    pub(crate) edit_cursor_tick_frame: Option<u64>,
    /// `C4Game::FileMonitor`. Armed once per game when
    /// `Developer.AutoFileReload` is set and the app is windowed
    /// (`C4Game.cpp:2413-2424`), started after definitions have loaded.
    pub(crate) file_monitor: Option<clonk_platform::file_monitor::DirectoryMonitor>,
    /// `C4EditCursor::Hold` — set by a press, cleared by the release.
    pub(crate) edit_cursor_hold: bool,
    /// `C4EditCursor::DragFrame` with its `(X, Y)` press anchor and live
    /// `(X2, Y2)` corner, both in world coordinates. `Some` exactly while a
    /// rubber band is armed.
    pub(crate) edit_cursor_drag_frame: Option<((i32, i32), (i32, i32))>,
    /// `C4EditCursor::DoContextMenu`'s popup and the physical viewport whose
    /// window it belongs to, in that window's surface coordinates.
    ///
    /// C++ hands the menu to the OS — `TrackPopupMenu` blocks until an item is
    /// chosen (`C4EditCursor.cpp:597`) — and neither of its two bodies is
    /// compiled on the reference build. A winit window cannot host an OS
    /// popup, so the port draws it onto the viewport's own frame and keeps it
    /// here until the next click resolves it.
    pub(crate) console_viewport_context_menu: Option<(
        u64,
        clonk_frontend::developer_context_menu::ViewportContextMenu,
    )>,
    /// The viewport whose popup swallowed the last button press, so the
    /// release that completes that click is swallowed with it.
    ///
    /// C++ needs no such latch: `TrackPopupMenu` blocks and the GTK menu holds
    /// a pointer grab, so the whole click — press *and* release — happens
    /// inside the menu and `C4EditCursor::LeftButtonUp` never sees it. A
    /// painted popup gets the release afterwards, when it may already have
    /// closed, and running the edit cursor's release then would clear the
    /// `Hold` `GrabContents` sets (`C4EditCursor.cpp:649`).
    pub(crate) console_viewport_context_menu_grab: Option<u64>,
    /// `C4Console::ToolsDlg` and `PropertyDlg`'s shared `C4DevmodeDlg`
    /// notebook. The model owns every decision about the window
    /// ([`crate::developer_toolbox`]); the runner owns the window itself, so
    /// the effects it produces queue here until the event loop, which is the
    /// only place winit will build one, can apply them.
    pub(crate) developer_toolbox: crate::developer_toolbox::DeveloperToolbox,
    pub(crate) developer_toolbox_effects: Vec<crate::developer_toolbox::ToolboxEffect>,
    /// `C4ObjectListDlg`'s `window != nullptr` — the whole of its state.
    /// Everything the list draws is read from the snapshot at redraw, so
    /// unlike the toolbox there is no model to keep beside the window.
    pub(crate) developer_object_list_open: bool,
    /// The open `C4ComponentHost::ShowDialog`: which component, the host
    /// holding its committed bytes, and the text being edited. C++ keeps the
    /// host on `C4Game` (`Game.Script`, `Game.Title`, `Game.Info`) for the
    /// whole round; the port has no runtime host at all, so it loads one when
    /// the editor opens and hands its bytes to the save.
    pub(crate) developer_component_editor: Option<DeveloperComponentEdit>,
    /// Components the user has committed this round, which the scenario save
    /// projects onto its group journal
    /// (`developer_console_save::component_save_mutations`). C++ keeps them on
    /// `C4Game` and asks each one at save time; the port collects them here as
    /// they are accepted, which is the same set for the same reason.
    pub(crate) developer_component_hosts: Vec<clonk_engine::developer_components::ComponentHost>,
    /// Native `C4Console::Editing` starts true and is irreversibly cleared
    /// when `EnableControls` observes a no-input playback. Opening another
    /// game defaults the edit cursor mode, but does not restore this latch.
    pub(crate) developer_console_editing_enabled: bool,
    pub(crate) developer_console_pointer: GuiPoint,
    /// Thread-safe tracing mirror drained by the console window each app
    /// iteration. It remains `None` for the fullscreen client.
    pub(crate) console_log_capture: Option<clonk_logging::ConsoleLogCapture>,
    /// `C4LogSystem::GuiSink`'s message-board attachment: the C4Script log
    /// stream the running board draws (`src/C4Log.cpp:226-240`). Fixtures that
    /// never install a subscriber leave it `None`.
    pub(crate) game_log_capture: Option<clonk_logging::GameLogCapture>,
    /// C4Game::fScriptCreatedObjects: set only when Scenario Initialize
    /// changed the live object count and cleared after the scenario-save
    /// double-object warning.
    pub(crate) script_created_objects: bool,
    /// Optional targeted crew-definition source for pathless sandbox
    /// fixtures. This is deliberately separate from `app_paths`: it must not
    /// make unrelated app subsystems appear install-initialized, but it does
    /// need to survive sandbox restart and saved-game restoration.
    pub(crate) sandbox_crew_definition_paths: Option<AppPaths>,
    pub(crate) configured_client_player_selection: Option<ConfiguredClientPlayerSelection>,
    pub(crate) material_library: Option<Arc<MaterialSet>>,
    // Fields drop in declaration order. Cancel an in-flight league request
    // before NetworkManager joins its worker so shutdown cannot wait for the
    // HTTP timeout.
    pub(crate) pending_lobby_internet_signup: Option<network::PendingMasterserverSignup>,
    pub(crate) pending_league_player_auth: Option<PendingLeaguePlayerAuth>,
    pub(crate) network_event_waker: Option<NetworkEventWakeCallback>,
    pub(crate) network: Option<NetworkManager>,
    pub(crate) network_mode: Option<NetworkMode>,
    /// Process-session credentials mutated by C4LeagueSignupDialog. Native
    /// persists LeagueAccount but deliberately keeps LeaguePassword only in
    /// memory, so never write this override through the INI helper.
    pub(crate) league_auth_session: Option<clonk_network::LeagueAuthRequestHead>,
    pub(crate) network_lobby: Option<NetworkLobbyState>,
    pub(crate) classic_host_lobby: Option<ClassicHostLobbyState>,
    pub(crate) lobby_preload_task: Option<LobbyPreloadTask>,
    pub(crate) lobby_preload_artifact: Option<LobbyPreloadArtifact>,
    pub(crate) network_start_wait: Option<NetworkStartWaitDialogState>,
    /// Host-owned `C4Network2::pLobbyCountdown` analogue. Packet-derived
    /// `NetworkLobbyState::countdown` is presentation only and never arms GO.
    pub(crate) host_lobby_countdown: Option<HostLobbyCountdown>,
    /// `Game.C4S.GetMinPlayer()` for the round this lobby is staging.
    ///
    /// C++ reads it off the loaded `C4Scenario` when the countdown expires
    /// (C4GameLobby.cpp:1163). The port has not applied a scenario while the
    /// lobby runs, so the host retains the staged head value here. `None` is
    /// "not known", and never aborts a round — an undetermined minimum must
    /// not be able to quit a server.
    pub(crate) network_lobby_min_players: Option<i32>,
    /// The live host session surfaces its locally submitted countdown once
    /// after broadcasting it. C++ instead applies that packet directly and
    /// excludes the host from the broadcast, so suppress exactly those echoes.
    pub(crate) pending_local_lobby_countdown_echoes: VecDeque<clonk_network::LobbyCountdownPacket>,
    pub(crate) lobby_ready_check_cooldown: LobbyReadyCheckCooldown,
    pub(crate) ready_check_toasts_enabled: bool,
    /// A pending "bring this window forward", drained by the runner.
    ///
    /// Distinct from the control channel's attention request, which is C++'s
    /// `FlashWindow` (`StdApp.h:283-288`) and only asks to be noticed. This
    /// one answers a click that already said "come back to the game", so it
    /// focuses instead of blinking.
    pub(crate) pending_window_attention: bool,
    pub(crate) pending_desktop_notifications:
        VecDeque<(DesktopNotificationId, DesktopNotification)>,
    /// Notifications the application has taken back, drained beside the
    /// shows. `ReadyCheckDialog::OnClosed` hides the dialog's toast from
    /// whichever side closed the prompt (`src/C4Network2.cpp:176-183`).
    pub(crate) pending_desktop_notification_dismissals: VecDeque<DesktopNotificationId>,
    /// The live ready check's toast, while it has one.
    pub(crate) live_ready_check_notification: Option<DesktopNotificationId>,
    /// Source of [`DesktopNotificationId`]s, so a dismissal names exactly the
    /// notification its check queued and never a later one.
    pub(crate) next_desktop_notification_id: u64,
    /// The live ready check's single-claim continuation.
    ///
    /// `C4Network2::ReadyCheckDialog` is one modal whose `ShowModalDlg` return
    /// value *is* the answer, so a toast button and the dialog can never both
    /// answer (`src/C4Network2.cpp:1672-1688`). The port's toast resolves on
    /// another thread, so that single return value becomes an atomic claim
    /// held here: whoever wins it owns the answer, and every other path —
    /// second button press, countdown expiry, teardown — becomes inert.
    /// `None` when no check is outstanding.
    pub(crate) lobby_ready_check_continuation:
        Option<crate::ready_check_notification::ReadyCheckContinuation>,
    /// Where a resolved continuation hides its toast.
    ///
    /// Held by the app rather than by the backend thread because
    /// `ReadyCheckDialog::OnClosed` hides the toast from whichever side
    /// resolved the prompt (`src/C4Network2.cpp:176-178`), including the
    /// in-window dialog. Defaults to a sink that shows nothing, which is also
    /// what a platform without a toast service leaves in place.
    pub(crate) lobby_ready_check_sink:
        std::sync::Arc<dyn crate::ready_check_notification::NotificationSink + Send + Sync>,
    pub(crate) control_messages: ControlMessageState,
    pub(crate) league_votes: LeagueVoteState,
    pub(crate) startup_network_connection: Option<StartupNetworkConnection>,
    pub(crate) classic_direct_reference_query: Option<ClassicDirectReferenceQuery>,
    /// Frozen C++-ordered address attempts retained across password prompts.
    pub(crate) pending_network_join: Option<ClientSettings>,
    pub(crate) staged_network_host_scenario: Option<StagedNetworkHostScenario>,
    pub(crate) sync_checks: SyncCheckState,
    pub(crate) network_ticks: NetworkTickGate,
    pub(crate) waiting_network_control: Option<NetworkControlWait>,
    /// When the current lockstep stall began, and whether it has been announced.
    ///
    /// A control stall is completely silent in C++: `DrawHoldMessages` prints
    /// only "Pause", and only for `HaltCount`, which a stall never sets. The
    /// world therefore freezes while rendering carries on at full frame rate,
    /// which is indistinguishable from a hang — the symptom behind
    /// legacyclonk/LegacyClonk#28, "network games stop randomly". There is no
    /// C++ behaviour to preserve here, so the port says something.
    pub(crate) network_stall_since: Option<(Instant, bool)>,
    /// Simulation frames executed since anything was last drawn, so a long
    /// catch-up cannot leave the screen frozen. See `NETWORK_RENDER_FLOOR_FRAMES`.
    pub(crate) frames_since_redraw: u32,
    pub(crate) network_control_retry_pending: bool,
    pub(crate) network_sync: NetworkSyncGate,
    /// `C4GameControl::Input` packets produced outside the simulation in a
    /// local game. They execute together at the next control-rate frame.
    pub(crate) offline_control_input: Vec<NetworkControl>,
    /// Offline counterpart of C4Game::HaltCount. Pause/Unpause assign 1/0,
    /// while native modal owners increment/decrement the same counted stack.
    /// Any nonzero value stops simulation but leaves event/render loops alive.
    pub(crate) offline_halt_count: i32,
    pub(crate) network_control_running: bool,
    /// App-owned counterpart of C4Network2's runtime fStatusReached state.
    /// The session owns acknowledgement consensus; the app owns driving
    /// simulation to this target and stopping exactly at the control boundary.
    pub(crate) runtime_network_status_barrier: Option<RuntimeNetworkStatusBarrier>,
    /// Live host `Game.Network.Status` projection for periodic references.
    /// This is distinct from the control barrier, which may still be waiting
    /// after ChangeGameStatus has already switched Paused/Running.
    pub(crate) host_reference_paused: bool,
    /// Last authoritative C4Network2Status control mode. Runtime chat can
    /// replace it through the same status barrier as C4Network2::SetCtrlMode.
    pub(crate) runtime_network_control_mode: Option<i32>,
    /// Control mode applied after the status acknowledgement. The F4 option
    /// keeps displaying this while a newer status is still pending.
    pub(crate) runtime_network_committed_control_mode: Option<i32>,
    /// Last acknowledged C4Network2Status. Native retains both status flags
    /// until the next status request resets them.
    pub(crate) runtime_network_committed_status: Option<clonk_network::NetworkStatus>,
    pub(crate) runtime_network_join_allowed: Option<bool>,
    /// Host-owned override of `Config.Network.NoRejoinAfterElimination`
    /// (clonk-org/clonk-rs#240). `None` reads the key, which the oracle has no
    /// counterpart for and which therefore defaults to readmitting.
    pub(crate) network_rejoin_after_elimination_allowed: Option<bool>,
    pub(crate) network_control_clock: Option<NetworkControlClock>,
    pub(crate) network_max_players: usize,
    pub(crate) network_is_league: bool,
    /// Exact synchronized `Game.Parameters.League` bytes. This is distinct
    /// from `network_is_league`, which models `isLeague()`'s LeagueAddress
    /// test; GetLeagueProgressData gates on this name instead.
    pub(crate) network_league_name: Vec<u8>,
    /// Process-local `Game.Parameters.StreamAddress`; this value is assigned
    /// by league Start but is intentionally absent from JoinData.
    pub(crate) network_stream_address: LegacyCString,
    /// C4Game::FPS and cFPS, sampled/reset by the one-second timer.
    pub(crate) frames_per_second: i32,
    pub(crate) frames_since_second: i32,
    /// Port-only presentation counters, sampled by the same one-second timer.
    /// C++ presents once per game tick, so `frames_per_second` is its render
    /// rate too; this port decouples the two and needs both numbers to say
    /// which half of a slow session is actually slow.
    pub(crate) presentation_stats: PresentationStats,
    pub(crate) input_latency_benchmark: Option<InputLatencyBenchmark>,
    /// C4Game::FullSpeed and FrameSkip are transient per-game scheduler
    /// controls. They are deliberately excluded from save capture/restore.
    pub(crate) full_speed: bool,
    pub(crate) frame_skip: i32,
    /// Frozen `C4GameParameters::AutoFrameSkip` for the active round. Unlike
    /// the startup option, this must not change while a game is running.
    pub(crate) auto_frame_skip: bool,
    /// Presentation detail the event loop's [`PresentationDetailGovernor`]
    /// currently allows. Presentation only — never consulted by simulation.
    pub(crate) presentation_detail: PresentationDetail,
    /// Process-local Config.Graphics.MaxRefreshDelay used by the application
    /// timer divisor. It is read once, then refreshed only after Options saves.
    pub(crate) max_refresh_delay_ms: u64,
    /// Ceiling for the startup timer alone. Equal to `max_refresh_delay_ms`
    /// unless `Graphics.SmoothPresentation` substituted the panel period, which
    /// deliberately leaves the game timer on the oracle value.
    pub(crate) startup_refresh_delay_ms: u64,
    /// Active monitor refresh period in whole milliseconds, once a window
    /// exists. Retained so an Options save can re-resolve the startup ceiling.
    pub(crate) display_refresh_period_ms: Option<u64>,
    /// C4Game::pNetworkStatistics exists for every running game. Only the
    /// Pings presentation tab is conditional on an enabled network session.
    pub(crate) network_stats: Option<NetworkStats>,
    pub(crate) network_stats_clients: HashSet<ClientId>,
    pub(crate) network_stats_players: HashSet<i32>,
    pub(crate) control_clients: ControlClientRegistry,
    /// `C4GameControlClient::iNextControl` for the activated-client copy.
    /// Native resets every entry to the current ControlTick whenever
    /// `CopyClientList` runs and never advances it afterwards.
    pub(crate) network_client_next_control_ticks: HashMap<i32, i32>,
    pub(crate) network_client_activity: NetworkClientActivity,
    pub(crate) control_player_infos: ControlPlayerInfoRegistry,
    /// Physical profile group retained for each locally admitted PlayerInfo.
    /// C4Player keeps this as `Filename`; the Rust control packet carries only
    /// a legacy presentation path, so retain the resolved path separately for
    /// `C4PlayerList::SynchronizeLocalFiles`.
    pub(crate) local_player_profile_paths: HashMap<i32, PathBuf>,
    /// `Application.NextMission` set by C4AbortGameDialog's Restart button.
    /// It deliberately survives a rejected/ignored league vote and is
    /// consumed by the next hard `QuitGame` route.
    pub(crate) abort_restart_pending: bool,
    /// The local profile and the rosters assembled around it.
    pub(crate) players: PlayerState,
    /// Armed on this client by the host's restart notice
    /// (`clonk_network::host_restart`). While it is armed, losing the host is a
    /// restart to follow rather than the dead host native assumes
    /// (src/C4Network2.cpp:1826-1832), so the round is torn down and the same
    /// address re-joined instead of dropping to local control.
    pub(crate) pending_host_rejoin: Option<PendingHostRejoin>,
    pub(crate) admission_resources: AdmissionResourceStore,
    pub(crate) blocking_resource_wait: Option<BlockingResourceWait>,
    pub(crate) aborted_player_resource_joins: HashSet<(i32, i32)>,
    /// Mutable host-owned JoinData used for lobby Set changes, GO
    /// activation, and later client admission. PreparedHostBootstrap remains
    /// the immutable resource proof from before the socket opened.
    pub(crate) host_join_snapshot: Option<clonk_network::HostJoinSnapshot>,
    /// C4Network2::fDynamicNeeded plus the clients waiting for the next
    /// synchronized runtime dynamic. One queued CID_Synchronize serves every
    /// request observed before that boundary.
    pub(crate) pending_runtime_dynamic_request: Option<PendingRuntimeDynamicRequest>,
    pub(crate) next_runtime_dynamic_save_generation: u64,
    pub(crate) pending_network_join_data: Option<clonk_network::JoinDataEnvelope>,
    /// Armed after a retained-session restart marker and consumed by the next
    /// JoinData. Initial admission also emits JoinData, so the marker must be
    /// tracked explicitly before acknowledging the lower restart fence.
    pub(crate) pending_round_restart_join_data: bool,
    pub(crate) initial_lobby_status_ack_pending: bool,
    pub(crate) client_start_barrier: ClientStartBarrier,
    pub(crate) pending_client_start_status: Option<clonk_network::NetworkStatus>,
    pub(crate) client_combined_scenario_path: Option<PathBuf>,
    pub(crate) client_combined_preload_file: ClientCombinedPreloadFile,
    /// Exact host-ordered NRT_Material groups from final resource publication
    /// or JoinData. `Some([])` is authoritative: neither side may fall back to
    /// a process-local global Material.c4g.
    pub(crate) network_material_resource_groups: Option<Vec<Group>>,
    pub(crate) executing_ready_tick: Option<Tick>,
    /// Control recording and playback.
    pub(crate) records: RecordingState,
    pub(crate) object_sprites: HashMap<String, DefinitionSprite>,
    pub(crate) sprite_cache: Arc<HashMap<String, DefinitionSprite>>,
    pub(crate) loading_state: Option<ScenarioLoadingState>,
    /// Successful activation leaves its 100% loader frame pending until one
    /// real window presentation accepts it. The game is already Running; this
    /// latch affects presentation only and never delays simulation or network
    /// readiness.
    pub(crate) terminal_loader_frame_pending: bool,
    pub(crate) boot_loading: Option<BootLoadingState>,
    /// When set, boot straight into the sandbox scenario once boot loading
    /// finishes (the `--sandbox` flag), instead of showing the menu. Cleared
    /// after the first auto-start so returning to the menu behaves normally.
    pub(crate) auto_start_sandbox: bool,
    /// A direct scenario waits for process boot resources before it starts
    /// either its local loader or prepared host, avoiding a race between the
    /// independent workers.
    pub(crate) auto_start_classic_command_line_scenario: bool,
    /// A `.c4u` package handed to the process on the command line.
    ///
    /// Kept apart from [`Self::update_check_requested`] because C++ applies an
    /// incoming package and *then* honours a requested check
    /// (`C4StartupMainDlg::OnShown`, cpp:259-269); they are not alternatives.
    pub(crate) incoming_update: Option<PathBuf>,
    /// A one-shot update check requested by `/update` or `clonk:update`.
    pub(crate) update_check_requested: bool,
    /// The update query in flight, if any (`C4UpdateDlg::CheckForUpdates`
    /// waiting on its `C4Network2VersionInfoClient`, cpp:280-300).
    pub(crate) update_check: Option<PendingUpdateCheck>,
    /// A release whose components are being downloaded and verified after the
    /// user accepted the update prompt.
    pub(crate) update_download: Option<PendingUpdateDownload>,
    /// The reusable `C4DownloadDlg` controller that owns this transfer's
    /// progress, its cancel semantics and its terminal error text.
    ///
    /// `C4UpdateDlg::DoUpdate` downloads through `C4DownloadDlg::DownloadFile`
    /// rather than presenting its own transfer UI (`C4UpdateDlg.cpp:165`), so
    /// the wrapper — not this module — composes `IDS_PRC_DOWNLOADERROR` and
    /// appends `IDS_MSG_UPDATENOTAVAILABLE` to a 404.
    pub(crate) update_download_dialog: Option<clonk_frontend::download_dialog::DownloadDialogState>,
    /// Whether showing the main menu may start the *automatic* check
    /// (`C4StartupMainDlg.cpp:270-275`).
    ///
    /// Off in test builds: a suite that opened a network-backed wait dialog on
    /// every app it boots would be testing the updater in every unrelated
    /// case. A test covering the boot path sets it explicitly. The manual
    /// command-line check is not gated by it.
    pub(crate) automatic_update_check_allowed: bool,
    /// Live keyboard and pointer state. Distinct from `input`, which is
    /// the binding dispatcher: this is what the devices last reported.
    pub(crate) live_input: InputState,
    /// C4GraphicsSystem::FreeScroll's process-presentation velocity and
    /// MostRecentScrolling clock. Repeated bare arrows carry the complete
    /// prior vector for 100ms without mutating deterministic player state.
    pub(crate) free_view_scroll_momentum: FreeViewScrollMomentum,
    /// Player menu whose title close button retained the current left-down.
    /// C4GUI::Button invokes only when that same button receives left-up.
    pub(crate) ingame_menu_close_pointer_capture: Option<i32>,
    /// Script menu close button retaining the current left-down.
    pub(crate) script_menu_close_pointer_capture: Option<(i32, ObjectId)>,
    /// Tooltip-style caption installed by a Help-mode object click or region
    /// hover, including C4MouseControl's move-count lifetime.
    pub(crate) ingame_mouse_help_caption: Option<IngameMouseHelpCaption>,
    pub(crate) mouse_state: Option<IngameButtonMouseState>,
    pub(crate) ingame_right_mouse_state: Option<IngameButtonMouseState>,
    /// C4Menu's retained drag element begins in GUI coordinates and becomes
    /// a C4MouseControl construction drag only after the menu sensitivity is
    /// crossed, so it cannot share the world-origin button state above.
    pub(crate) construction_menu_drag: Option<ConstructionMenuDrag>,
    /// C4MouseControl::Selection for object-only landscape frames. Unlike a
    /// crew frame, C++ retains this local list after button-up so a later
    /// object drag can issue Set + Append commands for the whole group.
    pub(crate) ingame_dragged_objects: Vec<ObjectId>,
    /// Platform-side C4MC_Button_LeftDouble synthesis for winit, whose
    /// MouseInput event does not expose an OS click count.
    pub(crate) ingame_last_left_down: Option<Instant>,
    /// C4MouseControl::LeftDoubleIgnoreUp.
    pub(crate) ingame_ignore_left_up: bool,
    pub(crate) window_active: bool,
    /// The display server has hidden the window outright — minimized, or fully
    /// covered by another one (`WindowEvent::Occluded`). C++ has no equivalent
    /// because Win32 deactivation minimizes its fullscreen window
    /// (C4FullScreen.cpp:139-145), so its inactive gate already covered this.
    /// Backends that cannot report occlusion leave it clear.
    pub(crate) window_occluded: bool,
    pub(crate) exit_requested: bool,
    /// Which exit ran, for the shutdown banner. Purely diagnostic and never
    /// cleared: `take_exit_request` consumes the latch above long before the
    /// loop unwinds, and the banner is written at the very end
    /// (clonk-org/clonk-rs#40).
    pub(crate) exit_reason: Option<&'static str>,
    /// A confirmed `Config.Default()` reset owns shutdown persistence. Keep
    /// this latched after `take_exit_request` so the event-loop tail cannot
    /// merge stale display values back into the freshly reset config.
    pub(crate) configuration_reset_requested: bool,
    pub(crate) game_over_dialog: Option<GameOverState>,
    pub(crate) game_over_handled: bool,
    pub(crate) pending_league_end: Option<PendingLeagueEnd>,
    /// `C4Game::InitKeyboard` reloads Extra.c4g/KeyConfig.txt once per game.
    /// Keep that ownership check separate from the process-global language
    /// table so a new round cannot reuse a stale accept/refusal.
    pub(crate) runtime_key_config_cache: OnceLock<std::result::Result<RuntimeKeyConfig, String>>,
    /// Process-start localization/encoding metadata needed by live flash
    /// producers. The active message itself is runtime-only, survives a
    /// GraphicsSystem resize, and is reset by Game::Default/new-game.
    pub(crate) runtime_flash_resources_cache:
        OnceLock<std::result::Result<RuntimeFlashResources, String>>,
    pub(crate) runtime_flash_message: Option<RuntimeFlashMessage>,
    /// Temporary player assigned to the existing primary viewport by replay
    /// `SetFilmView` or viewport cycling. The physical identity is unchanged.
    pub(crate) film_view_player: Option<i32>,
    /// Ordered `C4GraphicsSystem::Viewports` membership. Unlike the local
    /// control registry, this survives when `SetFilmView` retargets a
    /// viewport and the player that originally caused its creation leaves.
    pub(crate) physical_viewports: Vec<PhysicalViewportState>,
    /// Monotonic identity of the next concrete native-style viewport object.
    /// Player numbers may be reused while a film-retargeted older viewport
    /// remains alive, so player ownership cannot identify camera smoothing.
    pub(crate) next_physical_viewport_identity: u64,
    /// Ordinary owned/ownerless layouts may still be reconciled from the app's
    /// local-control registry. Once a film retarget makes physical lifetime
    /// observable, never reconstruct the concrete list until the game resets.
    pub(crate) physical_viewports_authoritative: bool,
    /// The runtime dialogs and the stack that orders them.
    pub(crate) dialogs: RuntimeDialogState,
    pub(crate) running_active_dialog: Option<RunningDialogStackEntry>,
    pub(crate) next_running_message_stack_id: u64,
    /// Native C4LeagueSignupDialog kept below its validation/cancellation
    /// MessageDialog while the current local player auth is suspended.
    pub(crate) league_signup_dialog: Option<PendingLeagueSignupDialog>,
    /// UserClose(false) blocks inside its cancellation notification before
    /// returning failure to LeaguePlrAuth. Reject the current player and
    /// resume the remaining players only after that notification closes.
    pub(crate) cancelled_league_signup_continuation: Option<LeaguePlayerAuthContinuation>,
    /// Keyboard-active z=+1 dialog. This can differ from the visual top while
    /// a z=+2 chat exists: inserting another message below chat does not call
    /// `Screen::ActivateDialog`.
    pub(crate) message_dialog_active_index: Option<usize>,
    /// Dialog whose button owns `CMouse::pDragElement`. A newer z=+1 dialog
    /// may be inserted above it while z=+2 chat keeps the old dialog active.
    pub(crate) message_dialog_pointer_capture_index: Option<usize>,
    /// The modal C4DefinitionSelDlg opened from the scenario book's
    /// "Choose definitions" checkbox. Its nested error message is kept in
    /// `message_dialogs`, so this controller remains alive underneath it.
    pub(crate) definition_selector: Option<clonk_frontend::definition_sel::DefinitionSelController>,
    /// Scenario/root retained until the selector accepts or cancels.
    pub(crate) pending_definition_selection: Option<PendingDefinitionSelection>,
    /// Local-client target and path/wire-name map for C4PlayerSelDlg.
    pub(crate) pending_lobby_player_selection: Option<PendingLobbyPlayerSelection>,
    /// Classic input dialog shared by startup prompts, game options, and the
    /// compact running-chat layout.
    pub(crate) game_option_input_dialog: Option<PendingGameOptionInputDialog>,
    /// `C4GUI::Screen::pContext`: the recursively open classic context-menu
    /// tree. The first caller is a startup player row; the chassis is shared
    /// by every later context-menu producer.
    pub(crate) context_menu: Option<ClassicContextMenu<AppContextMenuCommand>>,
    /// Player row whose C4GUI::ComboBox owns the shared context menu. This
    /// keeps the simple combo's arrow/highlight in its open phase.
    pub(crate) context_menu_lobby_team_player: Option<i32>,
    /// Core lobby/runtime option row whose C4GUI::ComboBox owns the shared
    /// context menu. The frontend retains this identity to render its open
    /// arrow phase.
    pub(crate) context_menu_lobby_option: Option<LobbyOptionKind>,
    /// Client row whose popup owns the shared context menu. A synchronized
    /// removal closes this menu before input can target a row that no longer
    /// exists.
    pub(crate) context_menu_lobby_kick_client: Option<i32>,
    /// Player row whose root context menu is open. An authoritative update
    /// that removes the row closes the stale popup before it can dispatch.
    pub(crate) context_menu_lobby_player: Option<(i32, i32, bool)>,
    /// C4GUI::ComboBox remembers the last menu index after Screen closes an
    /// outside-clicked menu. Retain that owner for the remainder of the same
    /// pointer-down so clicking the open combo closes instead of reopening it.
    pub(crate) context_menu_pointer_dismissed_lobby_team_player: Option<i32>,
    pub(crate) context_menu_pointer_dismissed_lobby_option: Option<LobbyOptionKind>,
    /// A context action may close on pointer-down. Retain that button until
    /// release so the underlying screen cannot receive a synthetic click.
    pub(crate) context_menu_pointer_capture: Option<ContextMenuPointerButton>,
    /// A modal may close on key-down. Retain consumed physical keys until
    /// their matching key-up so the underlying screen cannot activate.
    pub(crate) message_dialog_consumed_keys: HashSet<VirtualKeyCode>,
    pub(crate) league_signup_consumed_keys: HashSet<VirtualKeyCode>,
    pub(crate) league_signup_pointer_capture: bool,
    pub(crate) league_signup_pointer_position: Option<GuiPoint>,
    pub(crate) definition_selector_consumed_keys: HashSet<VirtualKeyCode>,
    /// Physical keys consumed by the join-address Edit until their matching
    /// release, even if a multiline paste moves focus back to the game list.
    pub(crate) netdlg_edit_consumed_keys: HashSet<VirtualKeyCode>,
    /// Retain a left/touch gesture if the selector closes before its matching
    /// release so the underlying scenario book cannot receive that release.
    pub(crate) definition_selector_pointer_capture: bool,
    pub(crate) game_option_input_consumed_keys: HashSet<VirtualKeyCode>,
    /// Physical pointer/gesture whose release belongs to the modal input
    /// dialog even if the dialog closes before that release arrives.
    pub(crate) game_option_input_pointer_capture: Option<ContextMenuPointerButton>,
    pub(crate) game_option_input_pointer_position: Option<GuiPoint>,
    pub(crate) game_option_input_last_click: Option<Instant>,
    pub(crate) game_option_consumed_keys: HashSet<VirtualKeyCode>,
    pub(crate) game_option_pointer_capture: bool,
    pub(crate) menu_backdrop_cache: StartupBackdropCache,
    /// Last definition-list label click for multi-selection double-click
    /// toggling (C4FileSelDlg::OnSelDblClick).
    pub(crate) definition_selector_last_click: Option<(usize, Instant)>,
    /// Last player-list row click (index, time) for forwarding the list box's
    /// double-click event (C4StartupPlrSelDlg.cpp:574-575).
    pub(crate) plrsel_last_click: Option<(usize, Instant)>,
    /// Last network-game row click for C4StartupNetDlg's list-box
    /// `OnSelDblClick -> DoOK` callback.
    pub(crate) netdlg_last_click: Option<(usize, Instant)>,
    /// Last join-address edit click for C4GUI::Edit's double-click word
    /// selection. This is independent from the game-list row gesture.
    pub(crate) netdlg_join_edit_last_click: Option<Instant>,
    /// C4MessageBoard's mode, LogBuffer cursor, and per-graphics-frame
    /// Fader/ScreenFader state.
    pub(crate) message_board: ClassicMessageBoardState,
    pub(crate) message_input_history: VecDeque<String>,
    /// `C4Player::ShowStartup` for the local player: device hint + name
    /// until the first control com (src/C4Player.cpp:1376,1735).
    pub(crate) show_startup_hint: bool,
    /// Developer-only `LC_APP_HUD_DEBUG=1` on an interactive debug build:
    /// draw FRAME/POS/VEL lines additively on top of the C++-faithful HUD.
    /// Launch classification keeps this false for parity and compatibility.
    pub(crate) debug_hud: bool,
}

/// The static layer of a startup view — the full-screen bilinear background
/// blit, or a whole parity-rendered screen — which is identical from frame
/// to frame. Re-renders restore it with a copy and draw only the dynamic
/// widgets on top, instead of re-running the per-pixel software blit.
#[derive(Default)]
pub(crate) struct StartupBackdropCache {
    pub(crate) key: Option<StartupBackdropKey>,
    pub(crate) pixels: Vec<u8>,
    /// Stable retained resource for graphical presentation. Keeping the same
    /// Surface identity lets the backend reuse the uploaded backdrop.
    pub(crate) retained: Option<Surface>,
}

/// Everything the static layer's pixels depend on.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct StartupBackdropKey {
    pub(crate) view: StartupView,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fair_crew: bool,
    pub(crate) record: bool,
    pub(crate) network_host_selector: bool,
}

/// Restores the cached static layer for `key` into `surface`, or renders it
/// through `render` and caches the result.
pub(crate) fn restore_or_render_backdrop(
    cache: &mut StartupBackdropCache,
    key: StartupBackdropKey,
    surface: &mut Surface,
    render: impl FnOnce(&mut Surface),
) {
    let cache_hit = cache.key == Some(key) && cache.pixels.len() == surface.pixels().len();
    if !cache_hit {
        let mut retained = Surface::new(surface.width(), surface.height(), surface.format());
        if surface.is_clonk_text_capture_active() {
            retained.begin_clonk_text_capture();
        }
        render(&mut retained);
        let _ = retained.take_clonk_text_capture();
        cache.key = Some(key);
        cache.pixels.clear();
        cache.pixels.extend_from_slice(retained.pixels());
        cache.retained = Some(retained);
    } else if cache.retained.is_none() {
        cache.retained = Surface::from_bytes(
            surface.width(),
            surface.height(),
            surface.format(),
            cache.pixels.clone(),
        )
        .ok();
    }

    if surface.is_gpu_scene_capture_active() {
        if let Some(retained) = cache.retained.as_ref() {
            let _ = surface.copy_transformed(
                retained,
                retained.bounds(),
                SurfacePoint::new(0, 0),
                &Transform::identity(),
            );
        }
    } else {
        surface.pixels_mut().copy_from_slice(&cache.pixels);
    }
}

pub(crate) struct RecordingTemplate {
    pub(crate) group: MutableGroup,
    pub(crate) output_path: PathBuf,
    pub(crate) initial_stream_chunk: Vec<u8>,
    pub(crate) runtime_seed: Option<RuntimeRecordingSeed>,
    pub(crate) description_title: Vec<u8>,
    pub(crate) description_definition_modules: Vec<Vec<u8>>,
}

#[derive(Clone)]
pub(crate) struct RuntimeRecordingSeed {
    /// `Game.Parameters.Scenario`: retained across FileSaveAs and used by
    /// C4Record::Start to derive the record basename.
    pub(crate) scenario_path: PathBuf,
    /// `Game.ScenarioFilename`: retargeted by FileSaveAs and copied by the
    /// non-initial C4GameSaveRecord.
    pub(crate) scenario_source_path: PathBuf,
    pub(crate) scenario_identifier: String,
    pub(crate) scenario_title: LegacyCString,
    pub(crate) definition_modules: Vec<String>,
    pub(crate) description_definition_modules: Vec<Vec<u8>>,
    /// Process-loaded SetModules path pair shared by the initial and every
    /// later non-initial record image.
    pub(crate) definition_executable_path: String,
    pub(crate) definition_path: String,
    pub(crate) scenario_origin: String,
    pub(crate) parameters: clonk_network::JoinGameParametersEnvelope,
    pub(crate) scenario_defaults: clonk_network::InitialNetworkScenarioDefaults,
}

#[derive(Clone, Copy)]
pub(crate) enum InitialRecordingSource<'a> {
    /// Fresh startup already captured this projection before
    /// `Script.Initialize`.
    Fresh(&'a clonk_engine::InitialNetworkGameData),
    /// A restored savegame must first materialize its virtual C4 save source,
    /// then capture the fInitial projection after native string/pointer
    /// enumeration has run.
    Loaded {
        music_enabled: bool,
        source_save_player_infos: Option<&'a [u8]>,
        source_title_png: Option<&'a [u8]>,
    },
}

pub(crate) struct RecordingSession {
    pub(crate) writer: ControlRecordWriter,
    pub(crate) ctrl_rec: File,
    pub(crate) output_path: PathBuf,
    pub(crate) league_streaming: bool,
    stream_writer_pos: usize,
    pub(crate) disk_writer_pos: usize,
    pub(crate) description_title: Vec<u8>,
    pub(crate) description_definition_modules: Vec<Vec<u8>>,
}

pub(crate) type StartupNetworkResult =
    std::result::Result<(NetworkMode, NetworkManager), NetworkStartError>;

pub(crate) struct StartupNetworkAttempt {
    cancellation: NetworkStartupCancellation,
    worker: Option<thread::JoinHandle<()>>,
}

impl StartupNetworkAttempt {
    fn cancel(&self) {
        self.cancellation.cancel();
    }

    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::error!("startup network launcher panicked during teardown");
            }
        }
    }
}

pub(crate) struct StartupNetworkConnection {
    pub(crate) receiver: Option<Receiver<StartupNetworkResult>>,
    pub(crate) selected_scenario: Option<(String, String)>,
    pub(crate) purpose: StartupNetworkPurpose,
    pub(crate) authenticated_league_players: Option<Vec<clonk_engine::ControlPlayerInfoEntry>>,
    attempt: Option<StartupNetworkAttempt>,
}

impl StartupNetworkConnection {
    pub(crate) fn new(
        receiver: Receiver<StartupNetworkResult>,
        selected_scenario: Option<(String, String)>,
        purpose: StartupNetworkPurpose,
    ) -> Self {
        Self {
            receiver: Some(receiver),
            selected_scenario,
            purpose,
            authenticated_league_players: None,
            attempt: None,
        }
    }

    pub(crate) fn with_attempt(mut self, attempt: StartupNetworkAttempt) -> Self {
        self.attempt = Some(attempt);
        self
    }

    pub(crate) fn finish_attempt(&mut self) {
        if let Some(mut attempt) = self.attempt.take() {
            attempt.join();
        }
    }

    fn cancel_attempt(&mut self) {
        if let Some(attempt) = self.attempt.as_ref() {
            attempt.cancel();
        }
        // If readiness won the cancellation race, dropping this receiver
        // makes the producer drop its newly constructed NetworkManager and
        // synchronously join the live transport before its launcher exits.
        self.receiver.take();
        if let Some(mut attempt) = self.attempt.take() {
            attempt.join();
        }
    }
}

impl Drop for StartupNetworkConnection {
    fn drop(&mut self) {
        self.cancel_attempt();
    }
}

pub(crate) fn spawn_startup_network_attempt<F>(
    name: &str,
    operation: F,
) -> io::Result<(Receiver<StartupNetworkResult>, StartupNetworkAttempt)>
where
    F: FnOnce(NetworkStartupCancellation) -> StartupNetworkResult + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let cancellation = NetworkStartupCancellation::new();
    let worker_cancellation = cancellation.clone();
    let worker = thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let result = operation(worker_cancellation);
            let _ = sender.send(result);
        })?;
    Ok((
        receiver,
        StartupNetworkAttempt {
            cancellation,
            worker: Some(worker),
        },
    ))
}

pub(crate) struct ClassicDirectReferenceQueryResult {
    pub(crate) settings: ClientSettings,
    pub(crate) password_needed: bool,
}

pub(crate) struct ClassicDirectReferenceQuery {
    pub(crate) receiver:
        Receiver<std::result::Result<ClassicDirectReferenceQueryResult, NetworkStartError>>,
}

impl RecordingSession {
    pub(crate) fn new(template: RecordingTemplate, league_streaming: bool, ctrl_rec: File) -> Self {
        Self {
            writer: ControlRecordWriter::new(),
            ctrl_rec,
            output_path: template.output_path,
            league_streaming,
            stream_writer_pos: 0,
            disk_writer_pos: 0,
            description_title: template.description_title,
            description_definition_modules: template.description_definition_modules,
        }
    }

    pub(crate) fn flush_control_delta(&mut self) -> io::Result<()> {
        let bytes = self.writer.bytes();
        debug_assert!(self.disk_writer_pos <= bytes.len());
        while self.disk_writer_pos < bytes.len() {
            match self.ctrl_rec.write(&bytes[self.disk_writer_pos..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to advance live CtrlRec",
                    ));
                }
                Ok(written) => self.disk_writer_pos += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        self.ctrl_rec.flush()
    }

    pub(crate) fn take_stream_delta(&mut self) -> Option<Vec<u8>> {
        if !self.league_streaming {
            return None;
        }
        let bytes = self.writer.bytes();
        debug_assert!(self.stream_writer_pos <= bytes.len());
        let delta = bytes[self.stream_writer_pos..].to_vec();
        self.stream_writer_pos = bytes.len();
        (!delta.is_empty()).then_some(delta)
    }
}

pub(crate) fn record_scenario_origin(
    scenario_path: &Path,
    app_paths: Option<&AppPaths>,
    fallback: &str,
) -> String {
    let relative = if scenario_path.is_relative() {
        Some(scenario_path.to_path_buf())
    } else {
        app_paths.and_then(|paths| {
            scenario_path
                .strip_prefix(paths.executable_data_root())
                .ok()
                .map(Path::to_path_buf)
        })
    };
    let origin = relative.map_or_else(
        || c4_filename_from_path(scenario_path),
        |path| path.to_string_lossy().replace('\\', "/"),
    );
    if origin.is_empty() {
        fallback.to_string()
    } else {
        origin
    }
}

pub(crate) fn clean_initial_record_group(group: &mut MutableGroup) {
    let stale_entries = group
        .entry_names()
        .into_iter()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".c4p")
                || lower == "title.bmp"
                || lower == "icon.bmp"
                || lower == "info.txt"
                || (lower.starts_with("title") && lower.ends_with(".txt"))
                || (lower.starts_with("desc") && lower.ends_with(".rtf"))
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for entry in stale_entries {
        group.remove_entry(&entry);
    }
}

/// Close the destination image after a failed record save, preserving every
/// component mutation completed before the failure. `C4GameSave` owns its
/// group while saving, so its destructor closes the partially written group
/// even when `Save` returns false.
fn persist_partial_recording_group(
    group: &MutableGroup,
    output_path: &Path,
    maker: &[u8],
) -> std::result::Result<(), String> {
    let mut partial = group.clone();
    if !maker.is_empty() {
        partial.set_maker_bytes_recursively(maker);
    }
    let packed = partial.pack().map_err(|error| error.to_string())?;
    replace_file_from_same_directory(output_path, &packed).map_err(|error| error.to_string())
}

pub(crate) fn partial_recording_failure(
    group: &MutableGroup,
    output_path: &Path,
    maker: &[u8],
    failure: String,
) -> String {
    match persist_partial_recording_group(group, output_path, maker) {
        Ok(()) => failure,
        Err(persist_error) => format!(
            "{failure}; additionally failed to close partial record {}: {persist_error}",
            output_path.display()
        ),
    }
}

fn c4_filename_from_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    let bytes = path.as_bytes();
    let mut filename_start = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte != b'/' && !(cfg!(windows) && byte == b'\\') {
            continue;
        }
        if index >= 4 && bytes[index - 4..index - 1].eq_ignore_ascii_case(b".c4") {
            return path[filename_start..].replace('\\', "/");
        }
        filename_start = index + 1;
    }
    path[filename_start..].replace('\\', "/")
}

pub(crate) fn recorded_player_resource_name(core: &clonk_engine::NetworkResourceCore) -> Vec<u8> {
    let basename = core
        .filename
        .as_bytes()
        .rsplit(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\'))
        .next()
        .unwrap_or_default();
    let mut target = core.id.to_string().into_bytes();
    target.push(b'-');
    target.extend_from_slice(basename);
    target
}

pub(crate) fn has_player_group_extension(name: &[u8]) -> bool {
    name.get(name.len().saturating_sub(4)..)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(b".c4p"))
}

pub(crate) fn path_to_legacy_bytes(path: &Path) -> Vec<u8> {
    resource_path_identity::path_wire_bytes(path)
}

pub(crate) fn path_as_legacy_text(path: &Path) -> String {
    clonk_script::c4_string_from_bytes(&path_to_legacy_bytes(path))
}

pub(crate) fn league_record_name(path: &Path) -> Option<LegacyCString> {
    LegacyCString::from_bytes(path_to_legacy_bytes(path))
}

pub(crate) fn raw_definition_description_modules(paths: &[PathBuf]) -> Vec<Vec<u8>> {
    paths
        .iter()
        .map(|path| path_to_legacy_bytes(path))
        .collect()
}

pub(crate) fn path_from_group_name_bytes(bytes: &[u8]) -> PathBuf {
    resource_path_identity::path_from_wire_bytes(bytes)
}

pub(crate) fn normalize_legacy_path_bytes(mut bytes: Vec<u8>) -> Vec<u8> {
    for byte in &mut bytes {
        if *byte == b'\\' {
            *byte = std::path::MAIN_SEPARATOR as u8;
        }
    }
    bytes
}

pub(crate) fn concatenate_legacy_path(prefix: &Path, suffix: &[u8]) -> PathBuf {
    let mut bytes = path_to_legacy_bytes(prefix);
    bytes.extend_from_slice(suffix);
    path_from_group_name_bytes(&normalize_legacy_path_bytes(bytes))
}

pub(crate) fn path_with_trailing_native_separator(path: &Path) -> PathBuf {
    let mut bytes = path_to_legacy_bytes(path);
    let separator = std::path::MAIN_SEPARATOR as u8;
    if bytes.last() != Some(&separator) {
        bytes.push(separator);
    }
    path_from_group_name_bytes(&bytes)
}

pub(crate) fn count_direct_stream_player_crew_files(
    group: &Group,
) -> std::result::Result<usize, String> {
    let mut direct_crew = 0;
    for entry in group.entries().map_err(|error| error.to_string())? {
        let is_crew = entry
            .name_bytes
            .get(entry.name_bytes.len().saturating_sub(4)..)
            .is_some_and(|extension| extension.eq_ignore_ascii_case(b".c4i"));
        if !is_crew {
            continue;
        }
        let child = if group.is_directory() {
            group.open_child(&entry.relative_path)
        } else {
            group.read_entry_bytes_exact(&entry).and_then(|bytes| {
                Group::from_raw_memory(path_from_group_name_bytes(&entry.name_bytes), bytes)
            })
        };
        let Ok(child) = child else {
            continue;
        };
        if child.read_file("ObjectInfo.txt").is_ok() {
            direct_crew += 1;
        }
    }
    Ok(direct_crew)
}

pub(crate) fn replay_control_record_chunks(
    group: &Group,
) -> std::result::Result<Vec<clonk_network::ControlRecordChunk>, String> {
    // C4Playback::Open tries LoadEntryString first. A successfully loaded text
    // entry is authoritative even when parsing fails; only a load failure
    // (including native's zero-sized-entry failure) selects CtrlRec.c4b.
    if let Ok(text) = group.load_entry_string("CtrlRec.txt") {
        return clonk_network::decode_control_record_text(&text)
            .map_err(|error| format!("has an invalid CtrlRec.txt stream: {error}"));
    }
    let binary = group
        .read_file("CtrlRec.c4b")
        .map_err(|error| format!("has no readable CtrlRec.c4b: {error}"))?;
    clonk_network::decode_control_record(&binary)
        .map_err(|error| format!("has an invalid CtrlRec.c4b stream: {error}"))
}

pub(crate) const SEARCH_EDIT_MAX_BYTES: usize = 254;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchCursorOperation {
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SearchEditState {
    pub(crate) text: String,
    pub(crate) caret: usize,
    pub(crate) anchor: usize,
    focused: bool,
    pub(crate) horizontal_scroll: i32,
    pub(crate) dragging: bool,
    /// C++ retains `iSelectionStart` independently from the visible caret,
    /// even when the selection is collapsed; an active drag reuses it.
    pub(crate) drag_anchor: usize,
    pub(crate) blink_ticks: u32,
    /// The IME composition in progress, drawn at the caret and never entered
    /// into `text` — only `Ime::Commit` reaches `insert_text`.
    composition: Option<clonk_frontend::ime::ImeComposition>,
}

impl SearchEditState {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn composition(&self) -> Option<&clonk_frontend::ime::ImeComposition> {
        self.composition.as_ref()
    }

    /// Replaces the composition in progress. `None` ends it, which is what
    /// `Ime::Commit` and `Ime::Disabled` both mean.
    pub(crate) fn set_composition(
        &mut self,
        composition: Option<clonk_frontend::ime::ImeComposition>,
    ) {
        self.composition = composition.filter(|composition| !composition.text.is_empty());
    }

    pub(crate) fn caret(&self) -> usize {
        self.caret
    }

    pub(crate) fn set_text(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        if text.len() > SEARCH_EDIT_MAX_BYTES {
            let mut end = SEARCH_EDIT_MAX_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        self.text = text;
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.horizontal_scroll = 0;
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    pub(crate) fn focus(&mut self) {
        if self.focused {
            return;
        }
        self.focused = true;
        self.anchor = 0;
        self.caret = self.text.len();
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    fn blur(&mut self) {
        self.focused = false;
        self.anchor = self.caret;
        self.dragging = false;
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    fn is_focused(&self) -> bool {
        self.focused
    }

    pub(crate) fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        (self.anchor != self.caret).then(|| {
            let start = self.anchor.min(self.caret);
            let end = self.anchor.max(self.caret);
            start..end
        })
    }

    fn selected_text(&self) -> Option<&str> {
        self.selection_range().map(|range| &self.text[range])
    }

    pub(crate) fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.drag_anchor = 0;
        self.blink_ticks = 0;
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        let start = range.start;
        self.text.replace_range(range, "");
        self.caret = start;
        self.anchor = start;
        self.drag_anchor = start;
        self.blink_ticks = 0;
        true
    }

    pub(crate) fn insert_text(&mut self, text: &str) -> bool {
        let selection_deleted = self.delete_selection();
        let available = SEARCH_EDIT_MAX_BYTES.saturating_sub(self.text.len());
        let mut sanitized = String::new();
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            let character = if character == '|' { '¦' } else { character };
            if sanitized.len() + character.len_utf8() > available {
                break;
            }
            sanitized.push(character);
        }
        if sanitized.is_empty() {
            return selection_deleted;
        }
        self.text.insert_str(self.caret, &sanitized);
        self.caret += sanitized.len();
        self.anchor = self.caret;
        self.blink_ticks = 0;
        true
    }

    /// `C4GUI::Edit::InsertText`: unlike keyboard input and ordinary Paste,
    /// the middle-button PRIMARY path inserts bytes without mapping `|` or
    /// treating line breaks as submit callbacks.
    pub(crate) fn insert_raw_text(&mut self, text: &str) -> bool {
        let old_text = self.text.clone();
        self.delete_selection();
        let available = SEARCH_EDIT_MAX_BYTES.saturating_sub(self.text.len());
        let mut end = text.len().min(available);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end > 0 {
            self.text.insert_str(self.caret, &text[..end]);
            self.caret += end;
            self.blink_ticks = 0;
        }
        self.anchor = self.caret;
        self.text != old_text
    }

    fn previous_boundary(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map(|character| at + character.len_utf8())
            .unwrap_or(self.text.len())
    }

    fn is_word_spacer(character: char) -> bool {
        character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
    }

    fn word_target(&self, direction: i32) -> usize {
        if direction < 0 {
            let mut cursor = self.caret;
            let mut nonspace_found = false;
            while cursor > 0 {
                let previous = self.previous_boundary(cursor);
                let character = self.text[previous..cursor]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    if nonspace_found {
                        break;
                    }
                } else {
                    nonspace_found = true;
                }
                cursor = previous;
            }
            cursor
        } else {
            let mut cursor = self.caret;
            let mut space_found = false;
            while cursor < self.text.len() {
                let next = self.next_boundary(cursor);
                let character = self.text[cursor..next]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    space_found = true;
                } else if space_found {
                    break;
                }
                cursor = next;
            }
            cursor
        }
    }

    pub(crate) fn move_cursor(
        &mut self,
        operation: SearchCursorOperation,
        ctrl: bool,
        shift: bool,
    ) {
        if self.selection_range().is_some() && !shift {
            self.anchor = self.caret;
            self.drag_anchor = 0;
        }
        let old_caret = self.caret;
        let target = match operation {
            SearchCursorOperation::Left => {
                if ctrl {
                    self.word_target(-1)
                } else {
                    self.previous_boundary(self.caret)
                }
            }
            SearchCursorOperation::Right => {
                if ctrl {
                    self.word_target(1)
                } else {
                    self.next_boundary(self.caret)
                }
            }
            SearchCursorOperation::Home => 0,
            SearchCursorOperation::End => self.text.len(),
        };
        if shift {
            if target != old_caret && self.selection_range().is_none() {
                self.anchor = old_caret;
                self.drag_anchor = old_caret;
            }
            self.caret = target;
        } else {
            self.caret = target;
            self.anchor = target;
        }
        self.blink_ticks = 0;
    }

    pub(crate) fn backspace(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if shift || self.caret == 0 {
            return false;
        }
        let start = if ctrl {
            self.word_target(-1)
        } else {
            self.previous_boundary(self.caret)
        };
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;
        self.blink_ticks = 0;
        true
    }

    pub(crate) fn delete(&mut self, ctrl: bool, shift: bool) -> bool {
        if self.delete_selection() {
            return true;
        }
        if shift || self.caret == self.text.len() {
            return false;
        }
        let end = if ctrl {
            self.word_target(1)
        } else {
            self.next_boundary(self.caret)
        };
        self.text.replace_range(self.caret..end, "");
        self.anchor = self.caret;
        self.blink_ticks = 0;
        true
    }

    pub(crate) fn scroll_cursor_in_view(
        &mut self,
        cursor_x: i32,
        client_width: i32,
        cursor_half: i32,
    ) {
        if client_width < 5 {
            return;
        }
        let cursor_x = cursor_x.saturating_add(cursor_half);
        if cursor_x < self.horizontal_scroll && self.horizontal_scroll > 0 {
            self.horizontal_scroll = cursor_x.saturating_sub(2).max(0);
        }
        if cursor_x > self.horizontal_scroll
            && cursor_x > client_width.saturating_add(self.horizontal_scroll)
        {
            self.horizontal_scroll =
                cursor_x.saturating_sub(client_width) + i32::from(self.caret < self.text.len()) * 2;
        }
    }

    pub(crate) fn tick_blink(&mut self) -> bool {
        if !self.focused {
            return false;
        }
        const BLINK_TICKS: u32 = 18;
        let before = (self.blink_ticks / BLINK_TICKS) % 2;
        self.blink_ticks = self.blink_ticks.wrapping_add(1);
        before != (self.blink_ticks / BLINK_TICKS) % 2
    }

    pub(crate) fn cursor_visible(&self) -> bool {
        self.focused && (self.blink_ticks / 18).is_multiple_of(2)
    }

    pub(crate) fn begin_pointer_selection(&mut self, position: usize) {
        let position = position.min(self.text.len());
        self.focus();
        self.anchor = position;
        self.caret = position;
        self.dragging = true;
        self.drag_anchor = position;
        self.blink_ticks = 0;
    }

    pub(crate) fn drag_pointer_selection(&mut self, position: usize) {
        if !self.dragging {
            return;
        }
        self.anchor = self.drag_anchor.min(self.text.len());
        self.caret = position.min(self.text.len());
        self.blink_ticks = 0;
    }

    pub(crate) fn end_pointer_selection(&mut self, position: usize) {
        if self.dragging {
            self.anchor = self.drag_anchor.min(self.text.len());
            self.caret = position.min(self.text.len());
            self.dragging = false;
            self.blink_ticks = 0;
        }
    }

    pub(crate) fn select_word_at(&mut self, mut position: usize) {
        position = position.min(self.text.len());
        if position < self.text.len() {
            let next = self.next_boundary(position);
            let character = self.text[position..next]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                if position == 0 {
                    return;
                }
                let previous = self.previous_boundary(position);
                let character = self.text[previous..position]
                    .chars()
                    .next()
                    .expect("non-empty character slice");
                if Self::is_word_spacer(character) {
                    return;
                }
                position = previous;
            }
        } else if position > 0 {
            let previous = self.previous_boundary(position);
            let character = self.text[previous..position]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                return;
            }
            position = previous;
        } else {
            return;
        }
        let mut start = position;
        while start > 0 {
            let previous = self.previous_boundary(start);
            let character = self.text[previous..start]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                break;
            }
            start = previous;
        }
        let mut end = self.next_boundary(position);
        while end < self.text.len() {
            let next = self.next_boundary(end);
            let character = self.text[end..next]
                .chars()
                .next()
                .expect("non-empty character slice");
            if Self::is_word_spacer(character) {
                break;
            }
            end = next;
        }
        self.anchor = start;
        self.caret = end;
        self.dragging = false;
        self.drag_anchor = start;
        self.blink_ticks = 0;
    }
}

pub(crate) fn scensel_search_context_entries(
    edit: &SearchEditState,
    clipboard_available: bool,
) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
    let mut entries = Vec::new();
    let selection = edit.selection_range();
    if selection.is_some() {
        entries.push(
            ContextMenuEntry::new("Cut")
                .with_tooltip("Moves the selection to the clipboard.")
                .with_action(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::Cut,
                )),
        );
        entries.push(
            ContextMenuEntry::new("Copy")
                .with_tooltip("Copies the selection to the clipboard.")
                .with_action(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::Copy,
                )),
        );
    }
    if clipboard_available {
        entries.push(
            ContextMenuEntry::new("Paste")
                .with_tooltip("Inserts the contents of the clipboard.")
                .with_action(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::Paste,
                )),
        );
    }
    if selection.is_some() {
        entries.push(
            ContextMenuEntry::new("Clear")
                .with_tooltip("Clears the selection.")
                .with_action(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::Clear,
                )),
        );
    }
    let whole_text_selected = selection
        .as_ref()
        .is_some_and(|range| range.start == 0 && range.end == edit.text().len());
    if !edit.text().is_empty() && !whole_text_selected {
        entries.push(
            ContextMenuEntry::new("Select all")
                .with_tooltip("Selects the complete text")
                .with_action(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::SelectAll,
                )),
        );
    }
    entries
}

pub(crate) fn clipboard_text_available() -> bool {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .is_ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn primary_clipboard_text() -> Option<String> {
    use arboard::{GetExtLinux, LinuxClipboardKind};

    arboard::Clipboard::new()
        .and_then(|mut clipboard| {
            clipboard
                .get()
                .clipboard(LinuxClipboardKind::Primary)
                .text()
        })
        .ok()
}

#[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
pub(crate) fn primary_clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .ok()
}

#[cfg(target_os = "windows")]
pub(crate) fn primary_clipboard_text() -> Option<String> {
    None
}

pub(crate) trait SelectionEdit {
    fn selected_text(&self) -> Option<&str>;
    fn delete_selection(&mut self) -> bool;
}

impl SelectionEdit for SearchEditState {
    fn selected_text(&self) -> Option<&str> {
        SearchEditState::selected_text(self)
    }

    fn delete_selection(&mut self) -> bool {
        SearchEditState::delete_selection(self)
    }
}

impl<Focus> SelectionEdit for RenameEdit<Focus> {
    fn selected_text(&self) -> Option<&str> {
        RenameEdit::selected_text(self)
    }

    fn delete_selection(&mut self) -> bool {
        RenameEdit::delete_selection(self)
    }
}

pub(crate) fn transfer_edit_selection<E, Edit: SelectionEdit>(
    edit: &mut Edit,
    cut: bool,
    transfer: impl FnOnce(&str) -> Result<(), E>,
) -> Result<bool, E> {
    let Some(selected) = edit.selected_text().map(str::to_string) else {
        return Ok(false);
    };
    transfer(&selected)?;
    if cut {
        edit.delete_selection();
    }
    Ok(true)
}

pub(crate) fn apply_scensel_search_paste(edit: &mut SearchEditState, text: &str) -> bool {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut segments = text.split('\n').peekable();
    while let Some(segment) = segments.next() {
        if segment.is_empty() && segments.peek().is_some() {
            continue;
        }
        if !segment.is_empty() {
            edit.insert_text(segment);
        }
        if segments.peek().is_some() && !segment.is_empty() {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClassicStartupSubscreen {
    Options(clonk_frontend::startup_options_dlg::OptionsSheet),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClassicStartupAction {
    OptionsAdvancedSettings,
    PlayerCrew { index: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClassicIngameMenuChild {
    NewPlayer,
    NetworkSurrender,
    ClientDisconnect,
    GoalInfo(String),
    RuleInfo(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunningChatMode {
    All,
    Allies,
    Say,
}

#[derive(Clone, Debug)]
pub(crate) struct RunningChatState {
    pub(crate) history_index: i32,
    pub(crate) active: bool,
    pub(crate) kind: RunningChatKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunningChatKind {
    Ordinary,
    MessageBoardInput(clonk_engine::ActiveMessageBoardInput),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeNetworkRole {
    Offline,
    Host,
    Client,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeveloperConsoleCompletionStyle {
    Win32,
    Gtk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeveloperConsoleCompletionEntry {
    Function(String),
    Separator,
}

/// Apply the native script-entry combo conventions to the engine-owned
/// autocomplete catalog. Win32 prepends scenario functions and a divider and
/// displays calls; GTK appends bare scenario names after the engine list.
pub(crate) fn developer_console_completion_entries(
    catalog: &clonk_engine::ConsoleScriptCompletionCatalog,
    style: DeveloperConsoleCompletionStyle,
) -> Vec<DeveloperConsoleCompletionEntry> {
    let function = |name: &str, calls: bool| {
        DeveloperConsoleCompletionEntry::Function(if calls {
            format!("{name}()")
        } else {
            name.to_string()
        })
    };
    match style {
        DeveloperConsoleCompletionStyle::Win32 => {
            let mut entries = catalog
                .scenario_functions
                .iter()
                .rev()
                .map(|name| function(name, true))
                .collect::<Vec<_>>();
            if !catalog.scenario_functions.is_empty() {
                entries.push(DeveloperConsoleCompletionEntry::Separator);
            }
            entries.extend(
                catalog
                    .engine_functions
                    .iter()
                    .map(|name| function(name, true)),
            );
            entries
        }
        DeveloperConsoleCompletionStyle::Gtk => catalog
            .engine_functions
            .iter()
            .chain(&catalog.scenario_functions)
            .map(|name| function(name, false))
            .collect(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeNetworkStatusBarrier {
    pub(crate) status: clonk_network::NetworkStatus,
    pub(crate) local_reached: bool,
    pub(crate) actual_control_tick: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStatusReachOutcome {
    NotReached,
    Reported,
    ReportFailed,
}

pub(crate) fn same_runtime_network_status_barrier(
    left: clonk_network::NetworkStatus,
    right: clonk_network::NetworkStatus,
) -> bool {
    left.state == right.state && left.target_tick == right.target_tick
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeHelpColumns {
    pub(crate) left: String,
    pub(crate) right: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeFlashResources {
    pub(crate) charset: RuntimeHelpCharset,
    pub(crate) music: String,
    pub(crate) speed: String,
    pub(crate) debug_mode: String,
    pub(crate) debug_mode_not_allowed: String,
    pub(crate) no_debug_mode: String,
    pub(crate) on: String,
    pub(crate) off: String,
    /// `C4FullScreen::ViewportCheck` flashes this when the last owned viewport
    /// closes and the ownerless observer viewport takes over
    /// (C4FullScreen.cpp:519-526).
    pub(crate) observer_menu: String,
}

impl RuntimeFlashResources {
    pub(crate) fn undefined() -> Self {
        let text = |key: &str| format!("[Undefined: {key}]");
        Self {
            charset: RuntimeHelpCharset::Windows1252,
            music: text("IDS_CTL_MUSIC"),
            speed: text("IDS_MSG_SPEED"),
            debug_mode: text("IDS_CTL_DEBUGMODE"),
            debug_mode_not_allowed: text("IDS_MSG_DEBUGMODENOTALLOWED"),
            no_debug_mode: text("IDS_MSG_NODEBUGMODE"),
            on: text("IDS_CTL_ON"),
            off: text("IDS_CTL_OFF"),
            observer_menu: text("IDS_MSG_PRESSORPUSHANYGAMEPADBUTT"),
        }
    }

    pub(crate) fn on_off(&self, label: &str, enabled: bool) -> String {
        format!("{label}: {}", if enabled { &self.on } else { &self.off })
    }

    pub(crate) fn music_on_off(&self, enabled: bool) -> String {
        self.on_off(&self.music, enabled)
    }

    pub(crate) fn debug_mode_on_off(&self, enabled: bool) -> String {
        self.on_off(&self.debug_mode, enabled)
    }

    pub(crate) fn speed(&self, frame_skip: i32) -> String {
        format_resource_string(self.speed.clone(), &[&frame_skip.to_string()])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeFlashMessage {
    pub(crate) text: String,
    pub(crate) remaining_draws: u16,
    pub(crate) y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimePhysicalKey {
    Keyboard(VirtualKeyCode),
    Gamepad { slot: u8, button: u8 },
    Raw(u32),
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeKeyChord {
    pub(crate) physical: RuntimePhysicalKey,
    pub(crate) modifiers: ModifiersState,
}

impl RuntimeKeyChord {
    pub(crate) fn keyboard(key: VirtualKeyCode, modifiers: ModifiersState) -> Self {
        Self {
            physical: RuntimePhysicalKey::Keyboard(key),
            modifiers,
        }
    }

    pub(crate) fn matches(self, key: VirtualKeyCode, modifiers: ModifiersState) -> bool {
        let c4_modifiers =
            modifiers & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if self.modifiers != c4_modifiers {
            return false;
        }
        match self.physical {
            RuntimePhysicalKey::Keyboard(configured) => configured == key,
            RuntimePhysicalKey::Raw(configured) => {
                input::encode_virtual_key_code(key).is_some_and(|raw| raw as u32 == configured)
            }
            RuntimePhysicalKey::Gamepad { .. } | RuntimePhysicalKey::Disabled => false,
        }
    }

    pub(crate) fn matches_gamepad_button(self, slot: u8, button: u8) -> bool {
        if !self.modifiers.is_empty() {
            return false;
        }
        match self.physical {
            RuntimePhysicalKey::Gamepad {
                slot: configured_slot,
                button: configured_button,
            } => {
                configured_slot == slot
                    && (configured_button == 10 + button
                        || configured_button == 0xff
                        || (configured_button == 0xfc && button < 4)
                        || (configured_button == 0xfb && button >= 4)
                        || (configured_button == 0xfe && button.is_multiple_of(2))
                        || (configured_button == 0xfd && !button.is_multiple_of(2)))
            }
            RuntimePhysicalKey::Keyboard(_)
            | RuntimePhysicalKey::Raw(_)
            | RuntimePhysicalKey::Disabled => false,
        }
    }

    pub(crate) fn matches_gamepad_direction(self, slot: u8, direction: ControlButton) -> bool {
        if !self.modifiers.is_empty() {
            return false;
        }
        let button = match direction {
            ControlButton::Left => 1,
            ControlButton::Up => 2,
            ControlButton::Right => 3,
            ControlButton::Down => 4,
        };
        matches!(
            self.physical,
            RuntimePhysicalKey::Gamepad {
                slot: configured_slot,
                button: configured_button,
            } if configured_slot == slot && configured_button == button
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RuntimeKeyConfig {
    pub(crate) overrides: BTreeMap<String, Vec<RuntimeKeyChord>>,
    pub(crate) net_observer_next_player: Vec<RuntimeKeyChord>,
    pub(crate) chart_toggle: Vec<RuntimeKeyChord>,
}

impl RuntimeKeyConfig {
    pub(crate) fn override_for(&self, name: &str) -> Option<&[RuntimeKeyChord]> {
        self.overrides.get(name).map(Vec::as_slice)
    }

    pub(crate) fn keyboard_override_matches(
        &self,
        name: &str,
        key: VirtualKeyCode,
        modifiers: ModifiersState,
    ) -> Option<bool> {
        self.override_for(name)
            .map(|codes| codes.iter().any(|code| code.matches(key, modifiers)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeGlobalKeyOutcome {
    Unhandled,
    UnhandledAfterDeniedDebug,
    Handled,
    DownstreamWithoutEngineDispatch,
}

/// A developer pane with a scroll bar of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeveloperPane {
    /// `C4PropertyDlg`'s output box.
    PropertyOutput,
    /// `C4ObjectListDlg`'s tree.
    ObjectList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeDebugKey {
    Mode,
    Vertices,
    ActionCycle,
    SolidMask,
}

/// The `C4ToolsDlg` actions registered at `KEYSCOPE_Console`
/// (`C4Game.cpp:3433-3439`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsoleToolsAction {
    /// `C4EditCursor::ToggleMode` (`C4Game.cpp:3432`).
    ToggleCursorMode,
    /// `C4EditCursor::Delete` (`C4Game.cpp:3440`).
    DeleteSelection,
    GradeUp,
    GradeDown,
    PopMaterial,
    PopTextures,
    ToggleIft,
    ToggleTool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCustomGamepadAction {
    Chat(RunningChatMode),
    Scoreboard,
    Abort,
    Pause,
    Chart,
    /// `C4Game::ToggleMusic` (C4Game.cpp:3370).
    Music,
    /// `C4Game::ToggleSound` (C4Game.cpp:3371).
    Sound,
    /// `C4GraphicsSystem::SaveScreenshot`; `true` is the `ScreenshotEx`
    /// full-landscape argument (C4Game.cpp:3372-3373).
    Screenshot {
        full_landscape: bool,
    },
    /// `C4Game::ToggleChat` — the external IRC dialog (C4Game.cpp:3374).
    ToggleChat,
    /// `C4GraphicsSystem::ToggleShowHelp` (C4Game.cpp:3377).
    Help,
    /// `C4MessageBoard::ControlScrollUp`/`Down` (C4Game.cpp:3381-3382).
    MessageBoardScroll {
        up: bool,
    },
    /// The four debug toggles registered between the message board and the
    /// playback-speed pair (C4Game.cpp:3385-3389). A refused toggle is not an
    /// error: the native callback returns false after its own flash text.
    Debug(RuntimeDebugKey),
    /// `C4GraphicsSystem::ViewportNextPlayer` in film view (C4Game.cpp:3415).
    FilmNextPlayer,
    /// `C4GraphicsSystem::FreeScroll` with its registered vector
    /// (C4Game.cpp:3423-3426).
    FreeViewScroll(Vector2),
    /// The console-scope `C4EditCursor`/`C4ToolsDlg` block, which the
    /// keyboard route resolves as [`ConsoleToolsAction`] (C4Game.cpp:
    /// 3432-3440). A gamepad reaches the same callbacks.
    ConsoleTools(ConsoleToolsAction),
    /// `C4Network2::ToggleClientListDlg` (C4Game.cpp:3379).
    ClientList,
    /// `C4GraphicsSystem::ViewportNextPlayer` at KEYSCOPE_FreeView — the
    /// observer registration of the callback `FilmNextPlayer` also names
    /// (C4Game.cpp:3443).
    ObserverNextPlayer,
    /// `C4GameControl::KeyAdjustControlRate` with its registered ±1
    /// (C4Game.cpp:3444-3445).
    ControlRate {
        delta: i32,
    },
    /// `C4Network2::ToggleAllowJoin` (C4Game.cpp:3446).
    AllowJoinToggle,
    /// `C4GraphicsSystem::ToggleShowNetStatus` (C4Game.cpp:3447).
    NetStatsToggle,
    /// The port-only `StatsToggle`, which registers after every C++ action.
    StatsToggle,
    SpeedUp,
    SpeedDown,
    Menu(ControlCommand),
    MenuOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeFlashProducerBoundary {
    ObserverPrompt,
    ObserverClear,
    RuntimeJoin,
    ControlRate,
    FairCrew,
}

impl RuntimeFlashProducerBoundary {
    pub(crate) const ALL: [Self; 5] = [
        Self::ObserverPrompt,
        Self::ObserverClear,
        Self::RuntimeJoin,
        Self::ControlRate,
        Self::FairCrew,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClassicScoreboardTrigger {
    UserToggle,
    ScriptVisibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScoreboardPointerTarget {
    Close,
    Title,
    Dialog,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScoreboardTitleDrag {
    pub(crate) pointer: GuiPoint,
    pub(crate) origin: (i32, i32),
}

/// Presentation-only state owned by the live native `C4ScoreboardDlg`.
/// The deterministic matrix/refcount remain in `clonk-engine`; this cache has
/// exactly the dialog lifetime and is rebuilt only by native `Update` events.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScoreboardDialogRuntime {
    pub(crate) presentation: Option<clonk_frontend::scoreboard::ScoreboardPresentationState>,
    pub(crate) layout_revision: u64,
    pub(crate) preferred: Option<clonk_frontend::classic_gui::IntRect>,
    pub(crate) pointer: Option<GuiPoint>,
    pub(crate) title_drag: Option<ScoreboardTitleDrag>,
    pub(crate) close_hovered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RunningDialogStackEntry {
    Scoreboard,
    RuntimeClientList,
    Message(u64),
    Chat,
}

impl RunningDialogStackEntry {
    pub(crate) fn z_order(self) -> i8 {
        match self {
            Self::Scoreboard | Self::RuntimeClientList => 0,
            Self::Message(_) => 1,
            Self::Chat => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClassicViewportBoundary {
    LocalViewportUnavailable { owner: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClassicParityBoundary {
    GlobalGuiBootstrapResources {
        issues: Vec<ClassicGuiBootstrapIssue>,
    },
    StartupBootstrapResources {
        issues: Vec<ClassicStartupBootstrapIssue>,
    },
    StartupSubscreen(ClassicStartupSubscreen),
    StartupAction(ClassicStartupAction),
    StartupModel {
        view: StartupView,
        missing: &'static str,
    },
    StartupStatusOverlay {
        view: StartupView,
        status: String,
    },
    StartupGameOver {
        view: StartupView,
    },
    StartupScreen {
        view: StartupView,
    },
    StartupMainResources {
        missing: Vec<&'static str>,
    },
    ScenarioStartInspection {
        path: PathBuf,
        detail: String,
    },
    IngameMenuResources {
        missing: Vec<&'static str>,
    },
    HudResources {
        missing: Vec<&'static str>,
    },
    IngameMenuDefinitionIcon {
        definition_id: String,
        detail: String,
    },
    GameOverResources {
        missing: Vec<String>,
    },
    GuiOverlayResources {
        overlay: &'static str,
        detail: String,
    },
    IngameMenuChild(ClassicIngameMenuChild),
    AppObjectMenu(AppObjectMenuMode),
    RuntimeHelpResources {
        detail: String,
    },
    RuntimeFlashResources {
        detail: String,
    },
    NetworkControlPacing {
        detail: String,
    },
    RuntimeKeyConfig {
        detail: String,
    },
    RuntimeAudioSystem {
        action: &'static str,
    },
    RuntimeFlashProducer(RuntimeFlashProducerBoundary),
    Scoreboard {
        trigger: ClassicScoreboardTrigger,
        rows: usize,
        columns: usize,
        show_count: i32,
    },
    RunningViewport(ClassicViewportBoundary),
    HudGameMessage {
        count: usize,
    },
    LoaderScreen {
        context: &'static str,
        detail: String,
    },
    GameLobby(ClassicGameLobbyBoundary),
}

impl fmt::Display for ClassicParityBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GlobalGuiBootstrapResources { issues } => write!(
                f,
                "classic process-global C4GUI bootstrap is unavailable ({}); refusing startup, loading, and running UI before mutation, cache, or pixels",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::StartupBootstrapResources { issues } => write!(
                f,
                "classic startup bootstrap is unavailable ({}); refusing every startup root before cache or pixels",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::StartupSubscreen(subscreen) => write!(
                f,
                "classic startup subscreen {subscreen:?} is not implemented; refusing incomplete Rust pane"
            ),
            Self::StartupAction(action) => write!(
                f,
                "classic startup action {action:?} is not implemented; refusing generic status or side effect"
            ),
            Self::StartupModel { view, missing } => write!(
                f,
                "classic startup model {missing} is unavailable for {view:?}; refusing generic Rust fallback"
            ),
            Self::StartupStatusOverlay { view, status } => write!(
                f,
                "classic startup status presentation is unavailable in {view:?}: {status}; refusing generic Rust status overlay"
            ),
            Self::StartupGameOver { view } => write!(
                f,
                "classic game-over evaluation dialog is running-mode only, but stale state reached startup view {view:?}; refusing a startup overlay or silent omission"
            ),
            Self::StartupScreen { view } => write!(
                f,
                "classic startup screen {view:?} is unavailable; refusing generic Rust fallback"
            ),
            Self::StartupMainResources { missing } => write!(
                f,
                "classic startup main-menu resources are unavailable (missing {})",
                missing.join(", ")
            ),
            Self::ScenarioStartInspection { path, detail } => write!(
                f,
                "cannot verify classic scenario-start constraints for {}: {detail}",
                path.display()
            ),
            Self::IngameMenuResources { missing } => write!(
                f,
                "classic in-game menu resources are unavailable (missing {})",
                missing.join(", ")
            ),
            Self::HudResources { missing } => write!(
                f,
                "classic HUD resources are unavailable (missing {}); refusing generic Rust fallback",
                missing.join(", ")
            ),
            Self::IngameMenuDefinitionIcon {
                definition_id,
                detail,
            } => write!(
                f,
                "classic in-game goal/rule symbol definition `{definition_id}` is unavailable: {detail}; refusing a blank symbol substitute"
            ),
            Self::GameOverResources { missing } => write!(
                f,
                "classic game-over resources are unavailable (missing {}); refusing generic Rust fallback",
                missing.join(", ")
            ),
            Self::GuiOverlayResources { overlay, detail } => write!(
                f,
                "classic {overlay} resources are unavailable: {detail}; refusing overlay construction or base pixels"
            ),
            Self::IngameMenuChild(child) => write!(
                f,
                "classic in-game menu child {child:?} is not implemented; refusing status/no-op substitute"
            ),
            Self::AppObjectMenu(mode) => write!(
                f,
                "classic app-owned object menu {mode:?} is unavailable; refusing generic Rust pane"
            ),
            Self::RuntimeHelpResources { detail } => write!(
                f,
                "classic runtime F1 help resources are unavailable: {detail}; refusing partial help pixels"
            ),
            Self::RuntimeFlashResources { detail } => write!(
                f,
                "classic timed flash-message resources are unavailable: {detail}; refusing partial flash pixels or producer mutation"
            ),
            Self::NetworkControlPacing { detail } => write!(
                f,
                "classic network control pacing is unavailable: {detail}; refusing a partial local TargetFPS mutation"
            ),
            Self::RuntimeKeyConfig { detail } => write!(
                f,
                "classic process-global key configuration is unavailable: {detail}; refusing to guess key ownership or dispatch"
            ),
            Self::RuntimeAudioSystem { action } => write!(
                f,
                "classic audio system is unavailable; refusing {action} and any partial option, playback, or flash mutation"
            ),
            Self::RuntimeFlashProducer(producer) => write!(
                f,
                "classic timed flash producer {producer:?} is unavailable because its authoritative runtime state is not modeled; refusing the producer action and partial flash mutation"
            ),
            Self::Scoreboard {
                trigger,
                rows,
                columns,
                show_count,
            } => write!(
                f,
                "classic C4ScoreboardDlg cannot render exact live data for {trigger:?} (rows={rows}, columns={columns}, show_count={show_count}); refusing partial scoreboard pixels"
            ),
            Self::RunningViewport(ClassicViewportBoundary::LocalViewportUnavailable { owner }) => {
                write!(
                    f,
                    "declared local player {owner} has no authoritative local viewport; refusing the solid navy fallback and an arbitrary first-object focus/selection"
                )
            }
            Self::HudGameMessage { count } => write!(
                f,
                "classic C4GameMessage renderer is unavailable for {count} visible message(s)"
            ),
            Self::LoaderScreen { context, detail } => write!(
                f,
                "classic C4LoaderScreen is unavailable during {context}: {detail}; refusing generic loading approximation"
            ),
            Self::GameLobby(ClassicGameLobbyBoundary::Resources { detail }) => write!(
                f,
                "classic game-lobby resources are unavailable: {detail}; refusing generic lobby"
            ),
            Self::GameLobby(ClassicGameLobbyBoundary::Model { detail }) => write!(
                f,
                "classic game-lobby model is unavailable: {detail}; refusing guessed lobby state"
            ),
            Self::GameLobby(ClassicGameLobbyBoundary::Child(child)) => write!(
                f,
                "classic game-lobby child {child:?} is not implemented; refusing generic child pane"
            ),
        }
    }
}

impl std::error::Error for ClassicParityBoundary {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HudMessageDrawability {
    NotDrawable,
    Drawable,
}

fn c4_object_local_int(object: &ObjectSnapshot, name: &str) -> i32 {
    // C4Value::getInt converts Int/Bool/nil and returns zero when conversion
    // fails. `as_c4_int` exposes exactly the deterministic stored cases; all
    // other snapshot variants therefore have the same zero result.
    object
        .local_vars
        .get(name)
        .and_then(|value| value.as_c4_int())
        .unwrap_or(0)
}

fn c4_parallax_view_coordinate(
    coordinate: i32,
    target: i32,
    logical_extent: i32,
    parallax: i32,
) -> i32 {
    if parallax == 0 && coordinate < 0 {
        coordinate + target + logical_extent
    } else {
        // Preserve C++'s operation order and signed integer truncation.
        coordinate - target * (parallax - 100) / 100
    }
}

pub(crate) fn c4_message_target_position(
    target: &ObjectSnapshot,
    offset: Vector2,
    shape_height: i32,
    viewport: ActiveViewportProjection,
) -> Vector2 {
    let (x, y) = if target.category & C4D_PARALLAX != 0 {
        (
            c4_parallax_view_coordinate(
                target.position.x,
                viewport.target_x,
                viewport.logical_width,
                c4_object_local_int(target, "__local_0"),
            ),
            c4_parallax_view_coordinate(
                target.position.y,
                viewport.target_y,
                viewport.logical_height,
                c4_object_local_int(target, "__local_1"),
            ),
        )
    } else {
        (target.position.x, target.position.y)
    };
    Vector2::new(x + offset.x, y + offset.y - shape_height / 2 - 5)
}

/// Shipped English for `IDS_PRC_ERRLOADER`, used when the language table
/// cannot be read (`planet/System.c4g/LanguageUS.txt:1227`).
pub(crate) const LOADER_INIT_FAILURE_TEXT: &str = "Error initializing loader screen.";

/// Why the classic loader screen is missing.
///
/// C++ separates these two and so must the port. `C4LoaderScreen::Init`
/// returning false is an ordinary fatal the native engine also produces: it
/// logs its own reason, and the caller logs `IDS_PRC_ERRLOADER` and stops
/// (`src/C4Application.cpp:241-247`; `src/C4Game.cpp:370-380`). Reporting that
/// as a parity boundary would claim the port has not implemented a loader
/// screen, when in fact it refused exactly where C++ refuses.
#[derive(Debug)]
pub(crate) enum LoaderScreenFailure {
    /// `Init` failed for a reason C++ has too — no loader matched the
    /// specification, or `Graphics.c4g` would not open.
    NativeInit(String),
    /// There is no install root to search. `Config.AtExePath` always yields one
    /// natively (`src/C4LoaderScreen.cpp:61`), so this is a port-only state,
    /// reached by path-less fixtures, and stays a parity boundary.
    NoInstallPath(String),
}

impl LoaderScreenFailure {
    pub(crate) fn detail(&self) -> &str {
        match self {
            Self::NativeInit(detail) | Self::NoInstallPath(detail) => detail,
        }
    }
}

pub(crate) fn report_classic_parity_boundary(
    error: ClassicParityBoundary,
) -> ClassicParityBoundary {
    tracing::error!(boundary = ?error, error = %error, "classic menu parity boundary reached");
    error
}

pub(crate) fn classic_parity_engine_error(error: ClassicParityBoundary) -> EngineError {
    EngineError::ClassicMenuParityBoundary {
        detail: error.to_string(),
    }
}

pub(crate) fn scenario_activation_engine_error(
    scenario_title: &str,
    error: EngineError,
) -> ScenarioActivationError {
    ScenarioActivationError::Recoverable(format!("Failed to start {scenario_title}: {error}"))
}

pub(crate) fn scenario_activation_scenario_error(
    scenario_title: &str,
    error: ScenarioError,
) -> ScenarioActivationError {
    ScenarioActivationError::Recoverable(format!("Failed to start {scenario_title}: {error}"))
}

pub(crate) fn classic_game_lobby_error(boundary: ClassicGameLobbyBoundary) -> anyhow::Error {
    anyhow::Error::new(report_classic_parity_boundary(
        ClassicParityBoundary::GameLobby(boundary),
    ))
}

pub(crate) fn classic_game_lobby_child_error(child: ClassicGameLobbyChild) -> EngineError {
    classic_parity_engine_error(report_classic_parity_boundary(
        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Child(child)),
    ))
}

pub(crate) fn classic_game_lobby_model_engine_error(detail: impl Into<String>) -> EngineError {
    classic_parity_engine_error(report_classic_parity_boundary(
        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Model {
            detail: detail.into(),
        }),
    ))
}

pub(crate) fn classic_ingame_menu_child_error(child: ClassicIngameMenuChild) -> EngineError {
    classic_parity_engine_error(report_classic_parity_boundary(
        ClassicParityBoundary::IngameMenuChild(child),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameSchedule {
    pub(crate) simulation_interval: Duration,
    pub(crate) refresh_interval: Duration,
    /// `None` denotes the startup timer. Running timers carry the unique
    /// engine/SetGameSpeed reset token even when their duration is unchanged.
    pub(crate) running_revision: Option<u64>,
}

/// C4Application's automatic `Game.DoSkipFrame` latch. A slow graphics pass
/// arms exactly one skip; consuming that skip clears the latch before any
/// later pass can arm it again (src/C4Application.cpp:463-476).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutomaticFrameSkip {
    skip_next_graphics: bool,
}

impl AutomaticFrameSkip {
    /// Returns whether this graphics pass should be skipped. Disabling the
    /// game parameter also clears any stale latch at a game-mode boundary.
    pub(crate) fn begin_graphics_pass(&mut self, enabled: bool) -> bool {
        if !enabled {
            self.skip_next_graphics = false;
            return false;
        }
        std::mem::take(&mut self.skip_next_graphics)
    }

    /// A manual/network `DoSkipFrame` consumes the same application pass as
    /// an automatic skip, so an armed automatic latch must not spill into the
    /// following graphics opportunity.
    pub(crate) fn consume_suppressed_graphics_pass(&mut self) {
        self.skip_next_graphics = false;
    }

    pub(crate) fn finish_graphics_pass(
        &mut self,
        enabled: bool,
        graphics_duration: Duration,
        game_tick_delay: Duration,
    ) {
        // Native uses `(pre_gfx + tick_delay) < now`, not `<=`.
        self.skip_next_graphics = enabled && graphics_duration > game_tick_delay;
    }
}

/// How much presentation detail a machine that cannot draw fast enough gives
/// up so the frame budget goes to simulation instead.
///
/// Both steps mirror something C++ exposes as static `[Graphics]` config
/// (`FireParticles`, `DisableGamma`); the divergence is choosing them
/// automatically from measured cost. The fire step also differs in *where* it
/// applies: C++'s switch nulls `pFire1`/`pFire2` so the engine stops emitting,
/// while this one skips the draw for a burning object's flames — the engine
/// must not depend on local frame timing, since its particle list feeds the
/// dev-replay hash. Script-created fire is untouched either way. Nothing here
/// reaches the simulation, so two clients at different detail levels stay in
/// lockstep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PresentationDetail {
    #[default]
    Full,
    /// Fire particles off. Each one is an independent unbatched draw call.
    NoFireParticles,
    /// ...and the monitor-gamma resolve dropped. That pass is a whole extra
    /// full-screen fill doing three dependent texture fetches per pixel, which
    /// is pure memory bandwidth on a tile-based GPU.
    NoGammaPass,
}

impl PresentationDetail {
    const LADDER: [Self; 3] = [Self::Full, Self::NoFireParticles, Self::NoGammaPass];

    fn step_down(self) -> Self {
        let rung = Self::LADDER.iter().position(|step| *step == self);
        rung.and_then(|rung| Self::LADDER.get(rung + 1).copied())
            .unwrap_or(self)
    }

    fn step_up(self) -> Self {
        let rung = Self::LADDER.iter().position(|step| *step == self);
        rung.filter(|rung| *rung > 0)
            .and_then(|rung| Self::LADDER.get(rung - 1).copied())
            .unwrap_or(Self::Full)
    }

    pub(crate) fn draws_fire_particles(self) -> bool {
        self == Self::Full
    }

    pub(crate) fn resolves_monitor_gamma(self) -> bool {
        self < Self::NoGammaPass
    }
}

/// Consecutive graphics passes over budget before detail is reduced. Long
/// enough that a shader compile or a texture upload cannot cost the player a
/// visual feature.
pub(crate) const DETAIL_STEP_DOWN_PASSES: u32 = 30;
/// Consecutive comfortable passes before detail is restored. Deliberately much
/// longer than the step down: re-adding cost is what caused the overrun.
pub(crate) const DETAIL_STEP_UP_PASSES: u32 = 120;
/// A pass only counts as comfortable below this share of the budget, so
/// recovery cannot oscillate around the threshold that triggered it.
const DETAIL_HEADROOM_PERCENT: u32 = 50;

/// Picks a [`PresentationDetail`] from measured graphics cost, with hysteresis
/// in both directions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentationDetailGovernor {
    detail: PresentationDetail,
    over_budget: u32,
    comfortable: u32,
}

impl PresentationDetailGovernor {
    pub(crate) fn detail(&self) -> PresentationDetail {
        self.detail
    }

    /// Feed one completed graphics pass. `enabled` mirrors the same
    /// `Graphics.AutoFrameSkip` game parameter that governs the existing
    /// automatic skip; turning it off restores full detail at once.
    pub(crate) fn record_graphics_pass(
        &mut self,
        enabled: bool,
        graphics_duration: Duration,
        budget: Duration,
    ) {
        if !enabled {
            *self = Self::default();
            return;
        }
        let comfortable_ceiling = budget
            .saturating_mul(DETAIL_HEADROOM_PERCENT)
            .checked_div(100)
            .unwrap_or(Duration::ZERO);
        if graphics_duration > budget {
            self.comfortable = 0;
            self.over_budget = self.over_budget.saturating_add(1);
            if self.over_budget >= DETAIL_STEP_DOWN_PASSES {
                self.over_budget = 0;
                self.detail = self.detail.step_down();
            }
        } else {
            // Both streaks are consecutive-pass counters, so a pass that fails
            // one side's test still has to break the other's streak. A pass in
            // the deadband between `comfortable_ceiling` and `budget` is
            // neither an overrun nor comfortable: it clears both.
            self.over_budget = 0;
            if graphics_duration <= comfortable_ceiling {
                self.comfortable = self.comfortable.saturating_add(1);
                if self.comfortable >= DETAIL_STEP_UP_PASSES {
                    self.comfortable = 0;
                    self.detail = self.detail.step_up();
                }
            } else {
                self.comfortable = 0;
            }
        }
    }
}

/// Keeps a slow machine drawing.
///
/// `AutomaticFrameSkip` thins graphics opportunities; it cannot help when the
/// starvation happens *inside* one application pass, because
/// `advance_simulation_pass` drains its accumulator without ever yielding. Two
/// presentation-only rules fix that:
///
///  * `simulation_burst_budget` bounds how long one pass may simulate before
///    returning to the event loop, sized so drawing keeps roughly
///    [`RENDER_RESERVE_PERCENT`] of the wall clock.
///  * `must_present` forces a repaint once [`MAX_TIME_BETWEEN_RENDERS`] has
///    elapsed, whatever the skip machinery decided.
///
/// Neither rule touches the engine: the same simulation frames run in the same
/// order, only spread across more application passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderFloor {
    last_graphics: Duration,
    last_presented: Option<Instant>,
    presented: bool,
}

impl RenderFloor {
    pub(crate) fn record_presentation(&mut self, at: Instant, graphics_duration: Duration) {
        self.last_graphics = graphics_duration;
        self.last_presented = Some(at);
        self.presented = true;
    }

    /// Whether a frame has ever reached the screen. Refusals and floor arming
    /// both move `last_presented` without drawing anything, so neither can
    /// answer this — and the inactive gate needs the honest answer to keep its
    /// hands off the frame that maps the window.
    pub(crate) const fn has_presented(&self) -> bool {
        self.presented
    }

    /// How long one `advance_simulation_pass` may run before yielding so the
    /// event loop can draw. Simulation gets `100 - RENDER_RESERVE_PERCENT` of
    /// the wall clock the last measured draw implies, never less than a single
    /// simulation period (progress must be guaranteed at any game speed) and
    /// never more than the repaint floor.
    pub(crate) fn simulation_burst_budget(&self, simulation_interval: Duration) -> Duration {
        let reserved = self
            .last_graphics
            .saturating_mul(100_u32.saturating_sub(RENDER_RESERVE_PERCENT))
            .checked_div(RENDER_RESERVE_PERCENT)
            .unwrap_or(MAX_TIME_BETWEEN_RENDERS);
        // `clamp` would panic for a game speed slower than the repaint floor.
        let ceiling = MAX_TIME_BETWEEN_RENDERS.max(simulation_interval);
        reserved.max(simulation_interval).min(ceiling)
    }

    /// A graphics opportunity the shell refused to draw at all — `Graphics.
    /// RenderInactive` withholding an inactive window (C4GraphicsSystem.cpp:96-106)
    /// — still consumed the opportunity, exactly like the automatic frame skip's
    /// `consume_suppressed_graphics_pass`. Re-arm the floor from it: there is no
    /// visible window to keep fresh, and a floor left latched makes every
    /// event-loop pass take an opportunity instead of one every 500 ms.
    ///
    /// The refusal costs no graphics time, so `last_graphics` is untouched.
    pub(crate) fn note_refused_presentation(&mut self, at: Instant) {
        self.last_presented = Some(at);
    }

    /// Whether the repaint floor is due. The first call arms the floor, so a
    /// session that has never drawn still gets its first frame on time.
    pub(crate) fn must_present(&mut self, now: Instant) -> bool {
        let since = *self.last_presented.get_or_insert(now);
        now.saturating_duration_since(since) >= MAX_TIME_BETWEEN_RENDERS
    }
}

/// Graphics-pass samples one second of [`PresentationStats`] may retain. Four
/// hundred covers any real present rate; the cap exists so a path that stops
/// driving the one-second timer cannot turn the accumulator into a leak.
pub(crate) const PRESENTATION_STATS_MAX_SAMPLES: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputLatencyBenchmark {
    interval: Duration,
    started: Option<Instant>,
    next_pair: Option<Instant>,
    pending: VecDeque<PendingInputLatencyBenchmark>,
    submitted_inputs: u64,
    latency_samples: Vec<Duration>,
    submitted_by_player: BTreeMap<i32, u64>,
    latency_samples_by_player: BTreeMap<i32, Vec<Duration>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingInputLatencyBenchmark {
    tick: Tick,
    control: InputLatencyControlKey,
    submitted_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputLatencyControlKey {
    player: i32,
    command: i32,
    data: i32,
    by_client: i32,
}

impl From<&clonk_engine::PlayerControlData> for InputLatencyControlKey {
    fn from(control: &clonk_engine::PlayerControlData) -> Self {
        Self {
            player: control.player,
            command: control.command,
            data: control.data,
            by_client: control.by_client,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputLatencyBenchmarkReport {
    pub(crate) elapsed: Duration,
    pub(crate) submitted_inputs: u64,
    pub(crate) executed_inputs: u64,
    pub(crate) pending_inputs: u64,
    pub(crate) p50: Duration,
    pub(crate) p95: Duration,
    pub(crate) p99: Duration,
    pub(crate) max: Duration,
    pub(crate) latency_samples: Vec<Duration>,
    pub(crate) players: Vec<InputLatencyBenchmarkPlayerReport>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputLatencyBenchmarkPlayerReport {
    pub(crate) player: i32,
    pub(crate) submitted_inputs: u64,
    pub(crate) executed_inputs: u64,
    pub(crate) pending_inputs: u64,
    pub(crate) p50: Duration,
    pub(crate) p95: Duration,
    pub(crate) p99: Duration,
    pub(crate) max: Duration,
    pub(crate) latency_samples: Vec<Duration>,
}

impl InputLatencyBenchmarkPlayerReport {
    pub(crate) fn machine_line(&self, elapsed: Duration) -> String {
        let samples = self
            .latency_samples
            .iter()
            .map(|sample| sample.as_nanos().to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "LC_APP_PRESENTATION_BENCHMARK_INPUT_PLAYER player={} elapsed_seconds={:.6} submitted_inputs={} executed_inputs={} pending_inputs={} input_latency_sample_count={} input_latency_p50_ms={:.6} input_latency_p95_ms={:.6} input_latency_p99_ms={:.6} input_latency_max_ms={:.6} input_latency_samples_ns=[{samples}]",
            self.player,
            elapsed.as_secs_f64(),
            self.submitted_inputs,
            self.executed_inputs,
            self.pending_inputs,
            self.latency_samples.len(),
            self.p50.as_secs_f64() * 1_000.0,
            self.p95.as_secs_f64() * 1_000.0,
            self.p99.as_secs_f64() * 1_000.0,
            self.max.as_secs_f64() * 1_000.0,
        )
    }
}

impl InputLatencyBenchmarkReport {
    pub(crate) fn machine_line(&self) -> String {
        let samples = self
            .latency_samples
            .iter()
            .map(|sample| sample.as_nanos().to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "LC_APP_PRESENTATION_BENCHMARK_INPUT elapsed_seconds={:.6} submitted_inputs={} executed_inputs={} pending_inputs={} input_latency_sample_count={} input_latency_p50_ms={:.6} input_latency_p95_ms={:.6} input_latency_p99_ms={:.6} input_latency_max_ms={:.6} input_latency_samples_ns=[{samples}]",
            self.elapsed.as_secs_f64(),
            self.submitted_inputs,
            self.executed_inputs,
            self.pending_inputs,
            self.latency_samples.len(),
            self.p50.as_secs_f64() * 1_000.0,
            self.p95.as_secs_f64() * 1_000.0,
            self.p99.as_secs_f64() * 1_000.0,
            self.max.as_secs_f64() * 1_000.0,
        )
    }
}

impl InputLatencyBenchmark {
    pub(crate) fn new(interval: Duration) -> Self {
        debug_assert!(!interval.is_zero());
        Self {
            interval,
            started: None,
            next_pair: None,
            pending: VecDeque::new(),
            submitted_inputs: 0,
            latency_samples: Vec::new(),
            submitted_by_player: BTreeMap::new(),
            latency_samples_by_player: BTreeMap::new(),
        }
    }

    pub(crate) fn start(&mut self, at: Instant) {
        if self.started == Some(at) {
            return;
        }
        self.started = Some(at);
        self.next_pair = Some(at);
        self.pending.clear();
        self.submitted_inputs = 0;
        self.latency_samples.clear();
        self.submitted_by_player.clear();
        self.latency_samples_by_player.clear();
    }

    pub(crate) fn pair_due(&mut self, now: Instant) -> bool {
        let Some(next_pair) = self.next_pair.filter(|next_pair| *next_pair <= now) else {
            return false;
        };
        let intervals_elapsed = now
            .saturating_duration_since(next_pair)
            .as_nanos()
            .checked_div(self.interval.as_nanos())
            .unwrap_or_default()
            .saturating_add(1);
        let advance = u32::try_from(intervals_elapsed)
            .ok()
            .and_then(|intervals| self.interval.checked_mul(intervals))
            .unwrap_or(self.interval);
        self.next_pair = Some(next_pair + advance);
        true
    }

    pub(crate) fn record_submission(
        &mut self,
        tick: Tick,
        control: &clonk_engine::PlayerControlData,
        submitted_at: Instant,
    ) {
        self.submitted_inputs = self.submitted_inputs.saturating_add(1);
        self.submitted_by_player
            .entry(control.player)
            .and_modify(|submitted| *submitted = submitted.saturating_add(1))
            .or_insert(1);
        self.pending.push_back(PendingInputLatencyBenchmark {
            tick,
            control: control.into(),
            submitted_at,
        });
    }

    pub(crate) fn record_execution(
        &mut self,
        tick: Tick,
        control: &clonk_engine::PlayerControlData,
        executed_at: Instant,
    ) -> bool {
        let control = control.into();
        let Some(pending) = self
            .pending
            .iter()
            .position(|pending| pending.tick == tick && pending.control == control)
            .and_then(|position| self.pending.remove(position))
        else {
            return false;
        };
        let latency = executed_at.saturating_duration_since(pending.submitted_at);
        self.latency_samples.push(latency);
        self.latency_samples_by_player
            .entry(control.player)
            .or_default()
            .push(latency);
        true
    }

    pub(crate) fn report(&self, elapsed: Duration) -> InputLatencyBenchmarkReport {
        let (p50, p95, p99) = graphics_pass_percentiles(&self.latency_samples);
        let players = self
            .submitted_by_player
            .iter()
            .map(|(player, submitted_inputs)| {
                let latency_samples = self
                    .latency_samples_by_player
                    .get(player)
                    .cloned()
                    .unwrap_or_default();
                let (p50, p95, p99) = graphics_pass_percentiles(&latency_samples);
                InputLatencyBenchmarkPlayerReport {
                    player: *player,
                    submitted_inputs: *submitted_inputs,
                    executed_inputs: latency_samples.len() as u64,
                    pending_inputs: self
                        .pending
                        .iter()
                        .filter(|pending| pending.control.player == *player)
                        .count() as u64,
                    p50,
                    p95,
                    p99,
                    max: latency_samples.iter().copied().max().unwrap_or_default(),
                    latency_samples,
                }
            })
            .collect();
        InputLatencyBenchmarkReport {
            elapsed,
            submitted_inputs: self.submitted_inputs,
            executed_inputs: self.latency_samples.len() as u64,
            pending_inputs: self.pending.len() as u64,
            p50,
            p95,
            p99,
            max: self
                .latency_samples
                .iter()
                .copied()
                .max()
                .unwrap_or_default(),
            latency_samples: self.latency_samples.clone(),
            players,
        }
    }
}

/// Live per-second presentation counters for the opt-in diagnostics overlay.
///
/// [`PresentationBenchmark`] already derives these numbers from the same
/// events, but only once, at the end of a fixed window, behind an environment
/// variable. This keeps them running for the whole session so the overlay can
/// report the half of the frame budget `frames_per_second` (C4Game::FPS)
/// structurally cannot see: that counter counts executed *game* frames, and
/// presentation in this port is deliberately decoupled from the game tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentationStats {
    presentations_since_second: u32,
    presentations_per_second: u32,
    skips_since_second: u32,
    skips_per_second: u32,
    last_graphics: Duration,
    graphics_samples: Vec<Duration>,
    graphics_p95: Duration,
}

impl PresentationStats {
    pub(crate) fn record_presentation(&mut self, graphics_duration: Duration) {
        self.presentations_since_second = self.presentations_since_second.saturating_add(1);
        self.last_graphics = graphics_duration;
        if self.graphics_samples.len() < PRESENTATION_STATS_MAX_SAMPLES {
            self.graphics_samples.push(graphics_duration);
        }
    }

    pub(crate) fn record_automatic_graphics_skip(&mut self) {
        self.skips_since_second = self.skips_since_second.saturating_add(1);
    }

    /// Close the current second, exactly as `C4Game::Sec1Timer` samples cFPS.
    pub(crate) fn sample_second(&mut self) {
        self.presentations_per_second = std::mem::take(&mut self.presentations_since_second);
        self.skips_per_second = std::mem::take(&mut self.skips_since_second);
        let samples = std::mem::take(&mut self.graphics_samples);
        if !samples.is_empty() {
            (_, self.graphics_p95, _) = graphics_pass_percentiles(&samples);
        }
    }

    pub(crate) const fn presentations_per_second(&self) -> u32 {
        self.presentations_per_second
    }

    pub(crate) const fn automatic_graphics_skips_per_second(&self) -> u32 {
        self.skips_per_second
    }

    pub(crate) const fn last_graphics(&self) -> Duration {
        self.last_graphics
    }

    pub(crate) const fn graphics_p95(&self) -> Duration {
        self.graphics_p95
    }

    pub(crate) fn retained_graphics_samples(&self) -> usize {
        self.graphics_samples.len()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PresentationBenchmarkMeasurement {
    started: Option<Instant>,
    simulation_frame: u64,
    runtime_stippels_at_start: usize,
    submissions: u64,
    retained_gpu_submissions: u64,
    cpu_submissions: u64,
    refreshed_frames: u64,
    automatic_graphics_skips: u64,
    graphics_total: Duration,
    graphics_max: Duration,
    graphics_samples: Vec<Duration>,
    /// The platform copy/present alone, taken out of the graphics pass it is
    /// timed inside. The software presenter copies the whole CPU frame here,
    /// so this is the term that separates composition cost from destination
    /// cost.
    present_samples: Vec<Duration>,
    /// The graphics pass with its present subtracted, per sample.
    raster_samples: Vec<Duration>,
    /// One sample per event-loop iteration, not per submission: a frame that
    /// skips its render still simulates, and the presenter's budget is only
    /// readable against the complete frame it has to fit inside.
    simulation_samples: Vec<Duration>,
    frame_samples: Vec<Duration>,
    /// Presentation-buffer reallocations, which are setup cost rather than
    /// ordinary frames. Kept out of `frame_samples` so a steady-state
    /// distribution cannot hide one.
    reallocation_samples: Vec<Duration>,
    retained_gpu_profiles: Vec<ReconciledRetainedGpuFrameProfile>,
    gpu_timestamp_frames: Vec<gpu_renderer::GpuTimestampFrame>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PresentationBenchmarkReport {
    pub(crate) elapsed: Duration,
    pub(crate) submissions: u64,
    pub(crate) retained_gpu_submissions: u64,
    pub(crate) cpu_submissions: u64,
    pub(crate) refreshed_frames: u64,
    pub(crate) simulation_frames: u64,
    pub(crate) runtime_stippels_at_start: usize,
    pub(crate) runtime_stippels_at_end: usize,
    pub(crate) automatic_graphics_skips: u64,
    pub(crate) graphics_average: Duration,
    pub(crate) graphics_max: Duration,
    pub(crate) graphics_p50: Duration,
    pub(crate) graphics_p95: Duration,
    pub(crate) graphics_p99: Duration,
    pub(crate) graphics_samples: Vec<Duration>,
    pub(crate) present_max: Duration,
    pub(crate) present_p50: Duration,
    pub(crate) present_p95: Duration,
    pub(crate) present_p99: Duration,
    pub(crate) present_samples: Vec<Duration>,
    pub(crate) raster_max: Duration,
    pub(crate) raster_p50: Duration,
    pub(crate) raster_p95: Duration,
    pub(crate) raster_p99: Duration,
    pub(crate) raster_samples: Vec<Duration>,
    pub(crate) simulation_max: Duration,
    pub(crate) simulation_p50: Duration,
    pub(crate) simulation_p95: Duration,
    pub(crate) simulation_p99: Duration,
    pub(crate) simulation_samples: Vec<Duration>,
    pub(crate) frame_max: Duration,
    pub(crate) frame_p50: Duration,
    pub(crate) frame_p95: Duration,
    pub(crate) frame_p99: Duration,
    pub(crate) frame_samples: Vec<Duration>,
    pub(crate) surface_reallocations: u64,
    pub(crate) reallocation_max: Duration,
    pub(crate) reallocation_p50: Duration,
    pub(crate) reallocation_p95: Duration,
    pub(crate) reallocation_p99: Duration,
    pub(crate) reallocation_samples: Vec<Duration>,
    pub(crate) retained_gpu_profiles: Vec<ReconciledRetainedGpuFrameProfile>,
    pub(crate) gpu_timestamp_frames: Vec<gpu_renderer::GpuTimestampFrame>,
}

impl PresentationBenchmarkReport {
    pub(crate) fn machine_line(&self) -> String {
        let elapsed_seconds = self.elapsed.as_secs_f64();
        let submission_fps = self.submissions as f64 / elapsed_seconds;
        let simulation_fps = self.simulation_frames as f64 / elapsed_seconds;
        let graphics_samples_ns = self
            .graphics_samples
            .iter()
            .map(|sample| sample.as_nanos().to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds={elapsed_seconds:.6} successful_present_submissions={} retained_gpu_present_submissions={} cpu_present_submissions={} presentation_submission_fps={submission_fps:.6} refreshed_frames={} simulation_frames={} simulation_fps={simulation_fps:.6} automatic_graphics_skips={} average_graphics_pass_ms={:.6} max_graphics_pass_ms={:.6} graphics_pass_sample_count={} graphics_pass_p50_ms={:.6} graphics_pass_p95_ms={:.6} graphics_pass_p99_ms={:.6} max_present_ms={:.6} present_p50_ms={:.6} present_p95_ms={:.6} present_p99_ms={:.6} max_raster_ms={:.6} raster_p50_ms={:.6} raster_p95_ms={:.6} raster_p99_ms={:.6} max_simulation_ms={:.6} simulation_p50_ms={:.6} simulation_p95_ms={:.6} simulation_p99_ms={:.6} max_frame_ms={:.6} frame_p50_ms={:.6} frame_p95_ms={:.6} frame_p99_ms={:.6} frame_sample_count={} surface_reallocations={} max_reallocation_ms={:.6} reallocation_p50_ms={:.6} reallocation_p95_ms={:.6} reallocation_p99_ms={:.6} graphics_pass_samples_ns=[{graphics_samples_ns}]",
            self.submissions,
            self.retained_gpu_submissions,
            self.cpu_submissions,
            self.refreshed_frames,
            self.simulation_frames,
            self.automatic_graphics_skips,
            self.graphics_average.as_secs_f64() * 1_000.0,
            self.graphics_max.as_secs_f64() * 1_000.0,
            self.graphics_samples.len(),
            self.graphics_p50.as_secs_f64() * 1_000.0,
            self.graphics_p95.as_secs_f64() * 1_000.0,
            self.graphics_p99.as_secs_f64() * 1_000.0,
            self.present_max.as_secs_f64() * 1_000.0,
            self.present_p50.as_secs_f64() * 1_000.0,
            self.present_p95.as_secs_f64() * 1_000.0,
            self.present_p99.as_secs_f64() * 1_000.0,
            self.raster_max.as_secs_f64() * 1_000.0,
            self.raster_p50.as_secs_f64() * 1_000.0,
            self.raster_p95.as_secs_f64() * 1_000.0,
            self.raster_p99.as_secs_f64() * 1_000.0,
            self.simulation_max.as_secs_f64() * 1_000.0,
            self.simulation_p50.as_secs_f64() * 1_000.0,
            self.simulation_p95.as_secs_f64() * 1_000.0,
            self.simulation_p99.as_secs_f64() * 1_000.0,
            self.frame_max.as_secs_f64() * 1_000.0,
            self.frame_p50.as_secs_f64() * 1_000.0,
            self.frame_p95.as_secs_f64() * 1_000.0,
            self.frame_p99.as_secs_f64() * 1_000.0,
            self.frame_samples.len(),
            self.surface_reallocations,
            self.reallocation_max.as_secs_f64() * 1_000.0,
            self.reallocation_p50.as_secs_f64() * 1_000.0,
            self.reallocation_p95.as_secs_f64() * 1_000.0,
            self.reallocation_p99.as_secs_f64() * 1_000.0,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentationPath {
    RetainedGpu,
    Cpu,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PresentationBenchmark {
    window: Duration,
    warmup_started: Option<Instant>,
    measurement: PresentationBenchmarkMeasurement,
    finished: bool,
}

/// The runtime benchmark begins after the first complete game execution, not
/// when network GO merely changes the application mode to Running. Native
/// queues initial player joins at GO, then executes controls before advancing
/// the first game frame (C4Network2Players.cpp:465-482; C4Game.cpp:796-801).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentationBenchmarkRuntimeReadiness {
    executed_frame: bool,
}

impl PresentationBenchmarkRuntimeReadiness {
    pub(crate) fn ready(&mut self, mode: AppMode) -> bool {
        if mode != AppMode::Running {
            self.executed_frame = false;
        }
        mode == AppMode::Running && self.executed_frame
    }

    pub(crate) fn observe(&mut self, mode: AppMode, executed_frames: u32) {
        if mode != AppMode::Running {
            self.executed_frame = false;
        } else if executed_frames > 0 {
            self.executed_frame = true;
        }
    }
}

impl PresentationBenchmark {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            window,
            warmup_started: None,
            measurement: PresentationBenchmarkMeasurement::default(),
            finished: false,
        }
    }

    pub(crate) fn poll(
        &mut self,
        running: bool,
        now: Instant,
        simulation_frame: u64,
    ) -> Option<PresentationBenchmarkReport> {
        self.poll_with_runtime_stippel_census(running, now, simulation_frame, || 0)
    }

    pub(crate) fn poll_with_runtime_stippel_census(
        &mut self,
        running: bool,
        now: Instant,
        simulation_frame: u64,
        runtime_stippel_census: impl FnOnce() -> usize,
    ) -> Option<PresentationBenchmarkReport> {
        if self.finished {
            return None;
        }
        if !running && self.measurement.started.is_none() {
            self.warmup_started = None;
            self.measurement = PresentationBenchmarkMeasurement::default();
            return None;
        }
        let warmup_started = *self.warmup_started.get_or_insert(now);
        let Some(started) = self.measurement.started else {
            if now.saturating_duration_since(warmup_started) >= PRESENTATION_BENCHMARK_WARMUP {
                self.measurement.started = Some(now);
                self.measurement.simulation_frame = simulation_frame;
                self.measurement.runtime_stippels_at_start = runtime_stippel_census();
            }
            return None;
        };
        let elapsed = now.saturating_duration_since(started);
        if elapsed < self.window {
            return None;
        }

        self.finished = true;
        let submissions = self.measurement.submissions;
        let graphics_average = if submissions == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(
                self.measurement.graphics_total.as_secs_f64() / submissions as f64,
            )
        };
        let graphics_samples = std::mem::take(&mut self.measurement.graphics_samples);
        let retained_gpu_profiles = std::mem::take(&mut self.measurement.retained_gpu_profiles);
        let gpu_timestamp_frames = std::mem::take(&mut self.measurement.gpu_timestamp_frames);
        let (graphics_p50, graphics_p95, graphics_p99) =
            graphics_pass_percentiles(&graphics_samples);
        let present_samples = std::mem::take(&mut self.measurement.present_samples);
        let raster_samples = std::mem::take(&mut self.measurement.raster_samples);
        let (present_p50, present_p95, present_p99) = graphics_pass_percentiles(&present_samples);
        let (raster_p50, raster_p95, raster_p99) = graphics_pass_percentiles(&raster_samples);
        let simulation_samples = std::mem::take(&mut self.measurement.simulation_samples);
        let frame_samples = std::mem::take(&mut self.measurement.frame_samples);
        let (simulation_p50, simulation_p95, simulation_p99) =
            graphics_pass_percentiles(&simulation_samples);
        let (frame_p50, frame_p95, frame_p99) = graphics_pass_percentiles(&frame_samples);
        let reallocation_samples = std::mem::take(&mut self.measurement.reallocation_samples);
        let (reallocation_p50, reallocation_p95, reallocation_p99) =
            graphics_pass_percentiles(&reallocation_samples);
        Some(PresentationBenchmarkReport {
            elapsed,
            submissions,
            retained_gpu_submissions: self.measurement.retained_gpu_submissions,
            cpu_submissions: self.measurement.cpu_submissions,
            refreshed_frames: self.measurement.refreshed_frames,
            simulation_frames: simulation_frame.saturating_sub(self.measurement.simulation_frame),
            runtime_stippels_at_start: self.measurement.runtime_stippels_at_start,
            runtime_stippels_at_end: runtime_stippel_census(),
            automatic_graphics_skips: self.measurement.automatic_graphics_skips,
            graphics_average,
            graphics_max: self.measurement.graphics_max,
            graphics_p50,
            graphics_p95,
            graphics_p99,
            graphics_samples,
            present_max: present_samples.iter().copied().max().unwrap_or_default(),
            present_p50,
            present_p95,
            present_p99,
            present_samples,
            raster_max: raster_samples.iter().copied().max().unwrap_or_default(),
            raster_p50,
            raster_p95,
            raster_p99,
            raster_samples,
            simulation_max: simulation_samples.iter().copied().max().unwrap_or_default(),
            simulation_p50,
            simulation_p95,
            simulation_p99,
            simulation_samples,
            frame_max: frame_samples.iter().copied().max().unwrap_or_default(),
            frame_p50,
            frame_p95,
            frame_p99,
            frame_samples,
            surface_reallocations: reallocation_samples.len() as u64,
            reallocation_max: reallocation_samples
                .iter()
                .copied()
                .max()
                .unwrap_or_default(),
            reallocation_p50,
            reallocation_p95,
            reallocation_p99,
            reallocation_samples,
            retained_gpu_profiles,
            gpu_timestamp_frames,
        })
    }

    pub(crate) fn measurement_window(&self) -> Option<(Instant, Instant)> {
        self.measurement
            .started
            .map(|started| (started, started + self.window))
    }

    pub(crate) fn record_successful_presentation(
        &mut self,
        now: Instant,
        graphics_duration: Duration,
        present_duration: Duration,
        refreshed: bool,
        path: PresentationPath,
    ) {
        self.record_successful_presentation_with_profile(
            now,
            graphics_duration,
            present_duration,
            refreshed,
            path,
            None,
        );
    }

    pub(crate) fn record_successful_retained_gpu_presentation(
        &mut self,
        now: Instant,
        graphics_duration: Duration,
        present_duration: Duration,
        refreshed: bool,
        profile: RetainedGpuFrameProfile,
    ) {
        self.record_successful_presentation_with_profile(
            now,
            graphics_duration,
            present_duration,
            refreshed,
            PresentationPath::RetainedGpu,
            Some(profile),
        );
    }

    fn record_successful_presentation_with_profile(
        &mut self,
        now: Instant,
        graphics_duration: Duration,
        present_duration: Duration,
        refreshed: bool,
        path: PresentationPath,
        retained_gpu_profile: Option<RetainedGpuFrameProfile>,
    ) {
        if self.finished {
            return;
        }
        let Some(started) = self.measurement.started else {
            return;
        };
        if now >= started + self.window {
            return;
        }
        self.measurement.submissions = self.measurement.submissions.saturating_add(1);
        match path {
            PresentationPath::RetainedGpu => {
                self.measurement.retained_gpu_submissions =
                    self.measurement.retained_gpu_submissions.saturating_add(1);
                if let Some(profile) = retained_gpu_profile {
                    self.measurement
                        .retained_gpu_profiles
                        .push(profile.reconcile(graphics_duration));
                }
            }
            PresentationPath::Cpu => {
                self.measurement.cpu_submissions =
                    self.measurement.cpu_submissions.saturating_add(1);
            }
        }
        self.measurement.refreshed_frames = self
            .measurement
            .refreshed_frames
            .saturating_add(u64::from(refreshed));
        self.measurement.graphics_total = self
            .measurement
            .graphics_total
            .saturating_add(graphics_duration);
        self.measurement.graphics_max = self.measurement.graphics_max.max(graphics_duration);
        self.measurement.graphics_samples.push(graphics_duration);
        self.measurement.present_samples.push(present_duration);
        // Saturating: the present is timed inside the pass, so it cannot
        // exceed it, but a clock that reports otherwise must not wrap.
        self.measurement
            .raster_samples
            .push(graphics_duration.saturating_sub(present_duration));
    }

    /// One event-loop iteration: how long the simulation burst took, and how
    /// long the whole frame took. Recorded for every iteration inside the
    /// measurement window, whether or not it presented.
    pub(crate) fn record_frame_pass(
        &mut self,
        now: Instant,
        simulation_duration: Duration,
        frame_duration: Duration,
    ) {
        if self.finished {
            return;
        }
        let Some(started) = self.measurement.started else {
            return;
        };
        if now >= started + self.window {
            return;
        }
        self.measurement
            .simulation_samples
            .push(simulation_duration);
        self.measurement.frame_samples.push(frame_duration);
    }

    /// One presentation-buffer reallocation, timed on its own.
    pub(crate) fn record_surface_reallocation(&mut self, now: Instant, duration: Duration) {
        if self.finished {
            return;
        }
        let Some(started) = self.measurement.started else {
            return;
        };
        if now >= started + self.window {
            return;
        }
        self.measurement.reallocation_samples.push(duration);
    }

    pub(crate) fn record_automatic_graphics_skip(&mut self) {
        if self.finished || self.measurement.started.is_none() {
            return;
        }
        self.measurement.automatic_graphics_skips =
            self.measurement.automatic_graphics_skips.saturating_add(1);
    }

    pub(crate) fn record_gpu_timestamp_frames(
        &mut self,
        frames: Vec<gpu_renderer::GpuTimestampFrame>,
    ) {
        if self.finished || self.measurement.started.is_none() {
            return;
        }
        self.measurement.gpu_timestamp_frames.extend(frames);
    }
}

pub(crate) fn graphics_pass_percentiles(samples: &[Duration]) -> (Duration, Duration, Duration) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let nearest_rank = |percentile: usize| {
        let index = sorted
            .len()
            .saturating_mul(percentile)
            .div_ceil(100)
            .saturating_sub(1);
        sorted.get(index).copied().unwrap_or_default()
    };
    (nearest_rank(50), nearest_rank(95), nearest_rank(99))
}

pub(crate) fn parse_presentation_benchmark_window(raw: &str) -> Option<Duration> {
    raw.parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

pub(crate) fn parse_presentation_benchmark_player_teams(raw: &str) -> Result<Vec<i32>, String> {
    if raw.is_empty() {
        return Err("benchmark player teams must not be empty".to_string());
    }
    raw.split(',')
        .map(|value| {
            value
                .parse::<i32>()
                .ok()
                .filter(|team| *team > 0)
                .ok_or_else(|| format!("benchmark player team `{value}` is not a positive ID"))
        })
        .collect()
}

pub(crate) fn presentation_benchmark_team_selection_controls(
    benchmark_active: bool,
    network_game: bool,
    players: &[i32],
    raw_teams: Option<&str>,
) -> Result<Vec<clonk_engine::InitScenarioPlayerControlData>, String> {
    if !benchmark_active || network_game {
        return Ok(Vec::new());
    }
    let Some(raw_teams) = raw_teams else {
        return Ok(Vec::new());
    };
    let teams = parse_presentation_benchmark_player_teams(raw_teams)?;
    if teams.len() != players.len() {
        return Err(format!(
            "benchmark configured {} player teams for {} pending players",
            teams.len(),
            players.len()
        ));
    }
    Ok(players
        .iter()
        .copied()
        .zip(teams)
        .map(
            |(player, team)| clonk_engine::InitScenarioPlayerControlData {
                team,
                player,
                by_client: 0,
            },
        )
        .collect())
}

pub(crate) fn parse_input_latency_benchmark_interval(raw: &str) -> Option<Duration> {
    raw.parse::<u64>()
        .ok()
        .filter(|milliseconds| *milliseconds > 0)
        .map(Duration::from_millis)
}

pub(crate) fn input_latency_benchmark_from_env() -> Option<InputLatencyBenchmark> {
    std::env::var_os(PRESENTATION_BENCHMARK_ENV)?;
    std::env::var(INPUT_LATENCY_BENCHMARK_INTERVAL_ENV)
        .ok()
        .and_then(|raw| parse_input_latency_benchmark_interval(&raw))
        .map(InputLatencyBenchmark::new)
}

pub(crate) fn presentation_benchmark_from_env() -> Option<PresentationBenchmark> {
    std::env::var(PRESENTATION_BENCHMARK_ENV)
        .ok()
        .as_deref()
        .and_then(parse_presentation_benchmark_window)
        .map(PresentationBenchmark::new)
}

pub(crate) fn presentation_benchmark_asserts_native_tick() -> bool {
    std::env::var(PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK_ENV).is_ok_and(|value| value == "1")
}

pub(crate) fn parse_presentation_benchmark_keep_running(raw: Option<&str>) -> bool {
    raw == Some("1")
}

pub(crate) fn presentation_benchmark_keeps_running() -> bool {
    parse_presentation_benchmark_keep_running(
        std::env::var(PRESENTATION_BENCHMARK_KEEP_RUNNING_ENV)
            .ok()
            .as_deref(),
    )
}

pub(crate) fn validate_native_tick_presentation_budget(
    report: &PresentationBenchmarkReport,
) -> std::result::Result<(), String> {
    if report.submissions == 0 || report.refreshed_frames == 0 {
        return Err("benchmark produced no refreshed presentation".to_string());
    }
    if report.cpu_submissions != 0 {
        return Err(format!(
            "CPU presentation submissions must be zero (observed {})",
            report.cpu_submissions
        ));
    }
    if report.retained_gpu_submissions != report.submissions {
        return Err(format!(
            "retained GPU submissions {} do not match total submissions {}",
            report.retained_gpu_submissions, report.submissions
        ));
    }
    let native_cadence =
        u64::try_from(report.elapsed.as_nanos() / INGAME_FRAME_INTERVAL.as_nanos())
            .unwrap_or(u64::MAX);
    if report.submissions < native_cadence {
        return Err(format!(
            "successful presentation submissions {} below native cadence {native_cadence}",
            report.submissions
        ));
    }
    if report.refreshed_frames < native_cadence {
        return Err(format!(
            "refreshed frames {} below native cadence {native_cadence}",
            report.refreshed_frames
        ));
    }
    if report.simulation_frames < native_cadence {
        return Err(format!(
            "simulation frames {} below native cadence {native_cadence}",
            report.simulation_frames
        ));
    }
    if report.automatic_graphics_skips != 0 {
        return Err(format!(
            "automatic graphics skips must be zero (observed {})",
            report.automatic_graphics_skips
        ));
    }
    if report.graphics_average > INGAME_FRAME_INTERVAL {
        return Err(format!(
            "average graphics pass {:.6}ms exceeds the native 28ms game tick",
            report.graphics_average.as_secs_f64() * 1_000.0
        ));
    }
    Ok(())
}

pub(crate) fn presentation_benchmark_context_line(
    runtime_players: usize,
    synchronized_player_infos: usize,
    activated_nonhost_clients: usize,
    runtime_crew_objects: usize,
    runtime_players_with_live_crew: usize,
    runtime_players_with_exactly_one_live_sf5b_crew: usize,
    runtime_stippels_at_start: usize,
    runtime_stippels_at_end: usize,
) -> String {
    format!(
        "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players={runtime_players} synchronized_player_infos={synchronized_player_infos} activated_nonhost_clients={activated_nonhost_clients} runtime_crew_objects={runtime_crew_objects} runtime_players_with_live_crew={runtime_players_with_live_crew} runtime_players_with_exactly_one_live_sf5b_crew={runtime_players_with_exactly_one_live_sf5b_crew} runtime_st5b_objects_at_measurement_start={runtime_stippels_at_start} runtime_st5b_objects_at_measurement_end={runtime_stippels_at_end}"
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PresentationBenchmarkNetworkEvidence {
    pub(crate) local_client_id: ClientId,
    pub(crate) preferred_message_route_peer_ids: Vec<ClientId>,
    pub(crate) tcp_preferred_message_routes: usize,
    pub(crate) udp_preferred_message_routes: usize,
    pub(crate) unknown_preferred_message_routes: usize,
    pub(crate) nonnegative_ping_peer_count: usize,
    pub(crate) nonnegative_lag_peer_count: usize,
    pub(crate) max_nonnegative_ping_ms: Option<i32>,
    pub(crate) max_nonnegative_lag_ms: Option<i32>,
    pub(crate) host_message_route_lag_ms: Option<i32>,
    pub(crate) max_packet_loss: u32,
    pub(crate) control_presend: i32,
    pub(crate) avg_control_send_time_us: i64,
}

impl PresentationBenchmarkNetworkEvidence {
    pub(crate) fn machine_line(&self) -> String {
        let peer_ids = self
            .preferred_message_route_peer_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=ok local_client_id={} preferred_message_route_peer_count={} preferred_message_route_peer_ids=[{peer_ids}] tcp_preferred_message_routes={} udp_preferred_message_routes={} unknown_preferred_message_routes={} nonnegative_ping_peer_count={} nonnegative_lag_peer_count={} max_nonnegative_ping_ms={} max_nonnegative_lag_ms={} host_message_route_lag_ms={} max_packet_loss={} control_presend={} avg_control_send_time_us={}",
            self.local_client_id,
            self.preferred_message_route_peer_ids.len(),
            self.tcp_preferred_message_routes,
            self.udp_preferred_message_routes,
            self.unknown_preferred_message_routes,
            self.nonnegative_ping_peer_count,
            self.nonnegative_lag_peer_count,
            self.max_nonnegative_ping_ms.unwrap_or(-1),
            self.max_nonnegative_lag_ms.unwrap_or(-1),
            self.host_message_route_lag_ms.unwrap_or(-1),
            self.max_packet_loss,
            self.control_presend,
            self.avg_control_send_time_us,
        )
    }
}

pub(crate) fn summarize_presentation_benchmark_network(
    local_client_id: ClientId,
    connections: &[clonk_network::RuntimeNetworkConnection],
    control_presend: i32,
    avg_control_send_time_us: i64,
) -> PresentationBenchmarkNetworkEvidence {
    let mut preferred_message_routes = BTreeMap::new();
    for connection in connections {
        if connection.usage.split('/').any(|usage| usage == "Msg") {
            preferred_message_routes
                .entry(connection.client_id)
                .or_insert(connection);
        }
    }
    let mut evidence = PresentationBenchmarkNetworkEvidence {
        local_client_id,
        preferred_message_route_peer_ids: preferred_message_routes.keys().copied().collect(),
        tcp_preferred_message_routes: 0,
        udp_preferred_message_routes: 0,
        unknown_preferred_message_routes: 0,
        nonnegative_ping_peer_count: 0,
        nonnegative_lag_peer_count: 0,
        max_nonnegative_ping_ms: None,
        max_nonnegative_lag_ms: None,
        host_message_route_lag_ms: None,
        max_packet_loss: 0,
        control_presend,
        avg_control_send_time_us,
    };
    for connection in preferred_message_routes.values() {
        match connection.protocol {
            clonk_network::NetworkProtocol::Tcp => {
                evidence.tcp_preferred_message_routes += 1;
            }
            clonk_network::NetworkProtocol::Udp => {
                evidence.udp_preferred_message_routes += 1;
            }
            _ => {
                evidence.unknown_preferred_message_routes += 1;
            }
        }
        if connection.ping_ms >= 0 {
            evidence.nonnegative_ping_peer_count += 1;
            evidence.max_nonnegative_ping_ms = Some(
                evidence
                    .max_nonnegative_ping_ms
                    .map_or(connection.ping_ms, |current| {
                        current.max(connection.ping_ms)
                    }),
            );
        }
        if connection.lag_ms >= 0 {
            evidence.nonnegative_lag_peer_count += 1;
            evidence.max_nonnegative_lag_ms = Some(
                evidence
                    .max_nonnegative_lag_ms
                    .map_or(connection.lag_ms, |current| current.max(connection.lag_ms)),
            );
        }
        if connection.client_id == 0 {
            evidence.host_message_route_lag_ms =
                (connection.lag_ms >= 0).then_some(connection.lag_ms);
        }
        evidence.max_packet_loss = evidence.max_packet_loss.max(connection.packet_loss);
    }
    evidence
}

pub(crate) fn inspect_presentation_benchmark_network(
    app: &GameApp,
) -> Option<std::result::Result<PresentationBenchmarkNetworkEvidence, String>> {
    let network = app.network.as_ref()?;
    let Some(control_clock) = app.network_control_clock else {
        return Some(Err("network_control_clock_unavailable".to_string()));
    };
    Some(
        network
            .runtime_connections()
            .map(|connections| {
                summarize_presentation_benchmark_network(
                    network.local_client_id(),
                    &connections,
                    control_clock.control_presend(),
                    control_clock.avg_control_send_time(),
                )
            })
            .map_err(|error| {
                tracing::error!(
                    %error,
                    "presentation benchmark runtime connection inspection failed"
                );
                "runtime_connection_inspection_failed".to_string()
            }),
    )
}

/// Count the distinct live objects retained by all runtime Crew lists. Native
/// MakeCrewMember requires active CrewMember objects before inserting them
/// (src/C4Player.cpp:1173-1203), and AssignDeath clears Alive before removing
/// the object from its player's crew (src/C4Object.cpp:1170-1200).
pub(crate) fn runtime_crew_object_count(snapshot: &SimulationSnapshot) -> usize {
    snapshot
        .players
        .iter()
        .flat_map(|player| player.crew.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|crew| {
            snapshot.object(*crew).is_some_and(|object| {
                object.crew_member && object.status.is_active() && object.alive
            })
        })
        .count()
}

pub(crate) fn runtime_stippel_object_count(snapshot: &SimulationSnapshot) -> usize {
    snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == "ST5B" && object.status.is_active())
        .count()
}

pub(crate) fn runtime_player_has_live_crew(snapshot: &SimulationSnapshot, player_id: i32) -> bool {
    snapshot
        .players
        .iter()
        .find(|player| player.id == player_id)
        .is_some_and(|player| {
            player.crew.iter().any(|crew| {
                snapshot.object(*crew).is_some_and(|object| {
                    object.owner == player_id
                        && object.crew_member
                        && object.status.is_active()
                        && object.alive
                })
            })
        })
}

pub(crate) fn runtime_players_with_live_crew(snapshot: &SimulationSnapshot) -> usize {
    snapshot
        .players
        .iter()
        .filter(|player| runtime_player_has_live_crew(snapshot, player.id))
        .count()
}

/// Count players whose Crew contains exactly one live, owner-matched SF5B.
/// HarpoonRace creates one SF5B owned by each player and calls MakeCrewMember
/// with it (HarpoonRace.c4s/Script.c:66-73); C++ retains that exact object in
/// the player's Crew (src/C4Player.cpp:1173-1203).
pub(crate) fn runtime_players_with_exactly_one_live_sf5b_crew(
    snapshot: &SimulationSnapshot,
) -> usize {
    snapshot
        .players
        .iter()
        .filter(|player| {
            player
                .crew
                .iter()
                .filter(|crew| {
                    snapshot.object(**crew).is_some_and(|object| {
                        object.definition_id == "SF5B"
                            && object.owner == player.id
                            && object.crew_member
                            && object.status.is_active()
                            && object.alive
                    })
                })
                .count()
                == 1
        })
        .count()
}

pub(crate) fn finish_presentation_benchmark(
    event_loop: &ActiveEventLoop,
    exit_code: &AtomicI32,
    report: PresentationBenchmarkReport,
    input_latency: Option<InputLatencyBenchmarkReport>,
    assert_native_tick: bool,
    runtime_players: usize,
    synchronized_player_infos: usize,
    activated_nonhost_clients: usize,
    runtime_crew_objects: usize,
    runtime_players_with_live_crew: usize,
    runtime_players_with_exactly_one_live_sf5b_crew: usize,
    network_evidence: Option<std::result::Result<PresentationBenchmarkNetworkEvidence, String>>,
    keep_running: bool,
) {
    println!("{}", report.machine_line());
    if let Some(input_latency) = input_latency {
        println!("{}", input_latency.machine_line());
        for player in &input_latency.players {
            println!("{}", player.machine_line(input_latency.elapsed));
        }
    }
    println!(
        "{}",
        presentation_benchmark_context_line(
            runtime_players,
            synchronized_player_infos,
            activated_nonhost_clients,
            runtime_crew_objects,
            runtime_players_with_live_crew,
            runtime_players_with_exactly_one_live_sf5b_crew,
            report.runtime_stippels_at_start,
            report.runtime_stippels_at_end,
        )
    );
    if let Some(network_evidence) = network_evidence {
        match network_evidence {
            Ok(network_evidence) => println!("{}", network_evidence.machine_line()),
            Err(error_code) => println!(
                "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=error error_code={error_code}"
            ),
        }
    }
    if assert_native_tick {
        if let Err(error) = validate_native_tick_presentation_budget(&report) {
            eprintln!("LC_APP_PRESENTATION_BENCHMARK result=fail error={error}");
            exit_code.store(2, AtomicOrdering::Relaxed);
            event_loop.exit();
            return;
        }
        println!("LC_APP_PRESENTATION_BENCHMARK result=pass native_tick_budget_ms=28");
    }
    if !keep_running {
        event_loop.exit();
    }
}

pub(crate) fn finish_app_presentation_benchmark(
    event_loop: &ActiveEventLoop,
    exit_code: &AtomicI32,
    app: &GameApp,
    report: PresentationBenchmarkReport,
    assert_native_tick: bool,
    keep_running: bool,
) {
    let network_evidence = inspect_presentation_benchmark_network(app);
    let input_latency = app
        .input_latency_benchmark
        .as_ref()
        .map(|benchmark| benchmark.report(report.elapsed));
    finish_presentation_benchmark(
        event_loop,
        exit_code,
        report,
        input_latency,
        assert_native_tick,
        app.engine.players().count(),
        app.control_player_infos.nonremoved_player_count(),
        app.control_clients
            .activated_client_ids()
            .into_iter()
            .filter(|client_id| *client_id != 0)
            .count(),
        runtime_crew_object_count(&app.snapshot),
        runtime_players_with_live_crew(&app.snapshot),
        runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot),
        network_evidence,
        keep_running,
    );
}

pub(crate) fn advance_graphics_deadline(
    deadline: Instant,
    now: Instant,
    interval: Duration,
) -> Instant {
    let next = deadline + interval;
    if next > now {
        // A pass that takes the graphics opportunity before the deadline is due
        // — the repaint floor, an immediate network retry — must not bank a
        // whole extra period, or a burst of them pushes the next timer frame
        // arbitrarily far out and only the floor keeps drawing.
        next.min(now + interval)
    } else {
        // Coalesce missed timer periods instead of issuing catch-up redraws.
        now + interval
    }
}

/// How long one `advance_simulation_pass` may simulate: the wall-clock share
/// [`RenderFloor::simulation_burst_budget`] reserves, and never past the moment
/// the next graphics opportunity comes due.
///
/// The reservation alone is the right bound only while the *simulation* is the
/// expensive half. It is derived from the last graphics pass, so an expensive
/// draw hands the simulation `(100 - RENDER_RESERVE_PERCENT)/RENDER_RESERVE_PERCENT`
/// times that cost before the pass will yield — the more drawing costs, the
/// longer it waits. Measured on a 33 ms pass that is 187 ms of simulation per
/// draw, which drops presentation onto [`MAX_TIME_BETWEEN_RENDERS`] while the
/// simulation holds full rate.
///
/// C++ never reaches that state: `C4Application::Execute` runs at most one
/// `Game.Execute()` per application pass and draws in the same pass
/// (C4Application.cpp:451-478), so drawing gets a slot every pass and a machine
/// that cannot keep up runs the *game* slow instead. Yielding at the deadline
/// restores that ordering without giving up catch-up: the pass still coalesces
/// every frame that fits before the next draw is due.
///
/// Determinism is unaffected for the same reason the reservation is: the budget
/// is only tested after a frame executed, so a pass always advances at least
/// one frame and unspent backlog stays in the accumulator.
pub(crate) fn simulation_burst_budget_before(
    reserved: Duration,
    now: Instant,
    next_graphics_deadline: Instant,
) -> Duration {
    reserved.min(next_graphics_deadline.saturating_duration_since(now))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NetworkControlPacing {
    pub(crate) behind: u32,
    pub(crate) overflow: bool,
    pub(crate) skip_render: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SimulationPassOutcome {
    pub(crate) did_update: bool,
    pub(crate) executed_frames: u32,
    pub(crate) skipped_render_frames: u32,
    pub(crate) skip_redraw: bool,
    pub(crate) immediate_network_retry: bool,
    /// The pass returned early to leave the event loop time to draw
    /// ([`RenderFloor::simulation_burst_budget`]). Backlog is retained, so the
    /// next pass continues from exactly the same frame.
    pub(crate) yielded_for_render: bool,
}

/// The refresh ceilings in force, one per application timer.
///
/// C++ has a single `Graphics.MaxRefreshDelay` because both of its timers are
/// welded to it. The port keeps `running_ms` on the oracle value and lets the
/// startup timer be subdivided independently, because only the startup timer is
/// actually timer-bound: see `smooth_presentation_subdivides_only_the_startup_timer`.
/// `From<u64>` keeps a bare ceiling meaning "both", so every C++-faithful
/// caller reads exactly as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RefreshCeilings {
    pub(crate) running_ms: u64,
    pub(crate) startup_ms: u64,
}

impl From<u64> for RefreshCeilings {
    fn from(max_refresh_delay_ms: u64) -> Self {
        Self {
            running_ms: max_refresh_delay_ms,
            startup_ms: max_refresh_delay_ms,
        }
    }
}

pub(crate) fn frame_schedule_for_mode(
    mode: AppMode,
    game_tick_delay_ms: u64,
    game_tick_delay_revision: u64,
    ceilings: impl Into<RefreshCeilings>,
) -> FrameSchedule {
    let ceilings = ceilings.into();
    match mode {
        AppMode::Menu | AppMode::Loading => FrameSchedule {
            simulation_interval: STARTUP_FRAME_INTERVAL,
            // The startup timer takes the same ceiling. The divisor is the
            // identity for every ceiling at or above 16 ms, so the native
            // default leaves the startup screens exactly as they were.
            refresh_interval: refresh_interval_for_tick(
                STARTUP_FRAME_INTERVAL.as_millis() as u64,
                ceilings.startup_ms,
            ),
            running_revision: None,
        },
        AppMode::Running => {
            let game_tick_delay_ms = game_tick_delay_ms.max(1);
            FrameSchedule {
                simulation_interval: Duration::from_millis(game_tick_delay_ms),
                refresh_interval: refresh_interval_for_tick(
                    game_tick_delay_ms,
                    ceilings.running_ms,
                ),
                running_revision: Some(game_tick_delay_revision),
            }
        }
    }
}

/// `C4Application::SetGameTickDelay`'s graphics divisor: keep graphics and
/// input wakeups responsive at slow tick rates by dividing the logic tick into
/// whole graphics periods no longer than the configured
/// `Graphics.MaxRefreshDelay` (C4Application.cpp:510-531). Subdividing changes
/// only how often the frame is presented; the caller's logic tick is untouched.
fn refresh_interval_for_tick(tick_ms: u64, max_refresh_delay_ms: u64) -> Duration {
    let tick_ms = tick_ms.max(1);
    let max_refresh_ms = max_refresh_delay_ms.max(1);
    let refresh_ms = if tick_ms < max_refresh_ms {
        tick_ms
    } else {
        tick_ms / tick_ms.div_ceil(max_refresh_ms)
    };
    Duration::from_millis(refresh_ms.max(1))
}

pub(crate) fn synchronize_frame_schedule(
    mode: AppMode,
    game_tick_delay_ms: u64,
    game_tick_delay_revision: u64,
    ceilings: impl Into<RefreshCeilings>,
    frame_schedule: &mut FrameSchedule,
    accumulator: &mut Duration,
) -> bool {
    let next_schedule =
        frame_schedule_for_mode(mode, game_tick_delay_ms, game_tick_delay_revision, ceilings);
    if next_schedule == *frame_schedule {
        return false;
    }
    *frame_schedule = next_schedule;
    *accumulator = Duration::ZERO;
    true
}

pub(crate) fn accumulate_frame_time_for_mode(
    mode: AppMode,
    game_tick_delay_ms: u64,
    game_tick_delay_revision: u64,
    ceilings: impl Into<RefreshCeilings>,
    frame_schedule: &mut FrameSchedule,
    accumulator: &mut Duration,
    elapsed: Duration,
) {
    // CStdApp::Execute drops a strict-more-than-two-second timer debt by
    // reanchoring LastExecute to now (StdAppUnix.cpp:261-284). The current
    // callback still runs, represented by one complete interval here. The
    // accumulator is elapsed time since the previous scheduled callback, so
    // account for its existing phase at the boundary.
    if accumulator.saturating_add(elapsed) > Duration::from_secs(2) {
        *accumulator = frame_schedule.simulation_interval;
    } else {
        // A speed of 1 means a full 1000 ms tick. Keep the runaway-catch-up
        // cap, but never clamp below one complete simulation interval.
        let accumulated_limit = MAX_ACCUMULATED_TIME.max(frame_schedule.simulation_interval);
        let clamped = elapsed.min(accumulated_limit);
        *accumulator = (*accumulator + clamped).min(accumulated_limit);
    }
    synchronize_frame_schedule(
        mode,
        game_tick_delay_ms,
        game_tick_delay_revision,
        ceilings,
        frame_schedule,
        accumulator,
    );
}

/// Executes one FullSpeed update, timer-paced updates, and any C++-style
/// network catch-up frames in one event-loop pass. Immediate frames do not
/// consume elapsed-time budget; a stalled pre-execution gate breaks the loop
/// instead of spinning.
pub(crate) fn advance_simulation_pass(
    app: &mut GameApp,
    frame_schedule: &mut FrameSchedule,
    accumulator: &mut Duration,
) -> Result<SimulationPassOutcome, EngineError> {
    advance_simulation_pass_within(app, frame_schedule, accumulator, Duration::MAX)
}

/// [`advance_simulation_pass`] bounded by a wall-clock budget.
///
/// A pass that exceeds `burst_budget` returns to the event loop instead of
/// draining the rest of its backlog, so drawing gets its reserved share on a
/// machine too slow to hold the tick budget ([`RenderFloor`]). The budget is
/// only ever checked *after* a frame executed, so simulation always advances.
/// Unspent backlog stays in `accumulator` and runs in the following passes:
/// the same frames execute in the same order either way.
pub(crate) fn advance_simulation_pass_within(
    app: &mut GameApp,
    frame_schedule: &mut FrameSchedule,
    accumulator: &mut Duration,
    burst_budget: Duration,
) -> Result<SimulationPassOutcome, EngineError> {
    let burst_started = Instant::now();
    let mut outcome = SimulationPassOutcome::default();
    let mut catch_up = false;
    let mut full_speed_due = app.mode == AppMode::Running && app.full_speed;
    let mut network_retry_due = app.take_network_control_retry();
    if full_speed_due {
        // FullSpeed is driven by one Winit Poll iteration at a time. Discard
        // timer debt so this pass cannot monopolize the event loop.
        *accumulator = Duration::ZERO;
    }

    loop {
        let timer_due = *accumulator >= frame_schedule.simulation_interval;
        if !timer_due && !catch_up && !full_speed_due && !network_retry_due {
            break;
        }

        let executed_interval = frame_schedule.simulation_interval;
        let immediate_network_retry =
            network_retry_due && !timer_due && !catch_up && !full_speed_due;
        let frame_before = app.engine.frame();
        if let Err(error) = app.update() {
            // Script errors during the simulation tick show in the log and
            // the game keeps running, like the C++ fail-safe exec; only
            // engine-internal errors are fatal.
            if matches!(error, EngineError::Script { .. }) {
                tracing::warn!(error = %error, "script error during tick; continuing like C++");
            } else {
                return Err(error);
            }
        }
        if timer_due {
            *accumulator -= executed_interval;
        }
        full_speed_due = false;
        network_retry_due = false;
        outcome.did_update = true;

        let frame_advanced = app.engine.frame() != frame_before;
        let pacing = if frame_advanced {
            app.network_control_pacing()
        } else {
            NetworkControlPacing::default()
        };
        let frame_skip = u64::try_from(app.frame_skip).unwrap_or(1);
        let manual_skip =
            frame_advanced && frame_skip > 1 && app.engine.frame().rem_euclid(frame_skip) != 0;
        let skip_render = pacing.skip_render || manual_skip;
        if frame_advanced {
            if immediate_network_retry {
                // CStdApp::Execute consumes DoNotDelay by anchoring LastExecute
                // to this packet-driven retry instead of retaining timer debt.
                *accumulator = Duration::ZERO;
                outcome.immediate_network_retry = true;
            }
            outcome.executed_frames = outcome.executed_frames.saturating_add(1);
            if skip_render {
                outcome.skipped_render_frames = outcome.skipped_render_frames.saturating_add(1);
            }
        }
        // Winit already coalesces intermediate catch-up frames into this one
        // pass, so the pass gets a single redraw decision for all of them. An
        // earlier skipped frame must not hide recovered state, and equally a
        // *final* skipped frame must not discard the render that the frames
        // before it asked for: taking only the last decision drew zero frames
        // for the whole burst whenever the burst happened to end on a skip.
        // C++ consumes DoSkipFrame per pass and keeps showing a fast-forward
        // during recovery (C4Application.cpp:463-476, C4GameControl.cpp:339),
        // so the pass draws unless every frame in it was a skip frame.
        outcome.skip_redraw =
            outcome.executed_frames > 0 && outcome.skipped_render_frames == outcome.executed_frames;
        catch_up = frame_advanced && pacing.overflow;

        let schedule_changed = synchronize_frame_schedule(
            app.mode,
            app.engine.game_tick_delay_ms(),
            app.engine.game_tick_delay_revision(),
            app.refresh_ceilings(),
            frame_schedule,
            accumulator,
        );
        if schedule_changed && !catch_up {
            break;
        }
        // Checked last, so every pass executes at least one frame before it
        // can yield. Overrunning here is normal on slow hardware: the frame
        // that spent the budget has already run.
        if burst_started.elapsed() >= burst_budget {
            outcome.yielded_for_render = true;
            break;
        }
    }

    apply_render_floor(app, &mut outcome);

    Ok(outcome)
}

/// Holds a floor under drawing during a long catch-up.
///
/// `pacing.skip_render` thins rendering hard while a client is behind — one
/// frame in twenty or worse — and because a pass coalesces several frames,
/// consecutive passes can each decide to draw nothing at all. A recovering
/// client then shows a frozen picture, which is exactly the "is it hung?"
/// symptom a silent stall produces. Spring solves the same problem the same
/// way, pinning draw to 2 Hz while fast-forwarding rather than giving the
/// simulation everything.
///
/// Applied after the pass has decided, so it overrides the skip without
/// perturbing the per-frame accounting that mirrors C++.
pub(crate) fn apply_render_floor(app: &mut GameApp, outcome: &mut SimulationPassOutcome) {
    app.frames_since_redraw = app
        .frames_since_redraw
        .saturating_add(outcome.executed_frames);
    if outcome.skip_redraw && app.frames_since_redraw >= NETWORK_RENDER_FLOOR_FRAMES {
        outcome.skip_redraw = false;
    }
    if !outcome.skip_redraw {
        app.frames_since_redraw = 0;
    }
}

pub(crate) struct MenuState {
    pub(crate) menu: StartupMenu,
    pointer_position: Option<GuiPoint>,
    pub(crate) stack: Vec<MenuLayer>,
    /// Entries from the current layer that survive the submitted search.
    /// C++ keeps the loader tree intact and rebuilds only the visible list
    /// (`C4StartupScenSelDlg::UpdateList`, cpp:1472-1538).
    visible_entries: Vec<FrontendScenario>,
    /// Parent-folder labels for flattened enhanced-search results, aligned
    /// with `visible_entries`. Ordinary folder browsing leaves these empty.
    visible_entry_contexts: Vec<Option<String>>,
    /// Current edit buffer/caret/selection. C++ does not filter until Enter
    /// invokes `OnSearchBarEnter`.
    pub(crate) search_edit: SearchEditState,
    /// Last submitted edit buffer used by `visible_entries`.
    pub(crate) applied_search_text: String,
    /// The richer product search is isolated from the C++ submit matcher.
    enhanced_search_active: bool,
    enhanced_search_total: usize,
    search_restore_selection: Option<String>,
    search_restore_scroll: Option<i32>,
    /// Inline `CallbackRenameEdit` projected over the selected row label.
    pub(crate) rename_edit: Option<ScenarioRenameState>,
    /// Logical-pixel offset of the left scenario `C4GUI::ListBox`.
    pub(crate) scenario_list_scroll: i32,
    /// Selection last passed through `ScrollRangeInView`. Keeping this
    /// separate prevents a later render from undoing deliberate wheel/thumb
    /// scrolling when the selection itself did not change.
    pub(crate) list_scroll_selection: Option<Option<usize>>,
    /// Logical-pixel offset of the right-page `C4GUI::TextWindow`.
    pub(crate) selection_info_scroll: i32,
    /// Captured classic scrollbar interaction. C++ retains the pin position
    /// while an arrow is held so one-pixel steps cannot stall on integer
    /// offset rounding (C4GuiContainers.cpp:391-473).
    pub(crate) scrollbar_interaction: Option<ScenselScrollbarInteraction>,
    /// Mutable state of the selection-dependent "Choose definitions"
    /// checkbox. C++ resets these flags from C4S only when selection changes;
    /// canceling the child selector retains a user toggle.
    pub(crate) definition_checkbox_enabled: bool,
    pub(crate) definition_checkbox_checked: bool,
    pub(crate) definition_checkbox_focused: bool,
    /// Recursive C4GUI dialog focus outside the embedded game-option window.
    /// The option controller owns the individual icon focus while this is
    /// `Options`; disabled icon buttons remain traversable like C++ buttons.
    dialog_focus: ScenselDialogFocus,
    /// Whether a synthetic "Back" row is injected at index 0. The network
    /// lobby's generic list uses it; the C++-faithful scenario book does not
    /// (C4StartupScenSelDlg has a Back *button*, no Back list entry).
    pub(crate) include_back: bool,
    /// FullscreenDialog creates the scenario title before the Tabular. A
    /// HideTitle map deletes it; returning to a book re-adds it at the end of
    /// the dialog, changing both presence and mouse-over z-order.
    pub(crate) scensel_title_present: bool,
    pub(crate) scensel_title_topmost: bool,
    enhanced_search_resources: EnhancedSearchResources,
}

#[derive(Clone, Debug)]
pub(crate) struct ScenarioRenameState {
    pub(crate) identifier: String,
    pub(crate) edit: RenameEdit<ScenselFocusSnapshot>,
    pub(crate) last_click: Option<Instant>,
}

#[derive(Clone, Debug)]
pub(crate) struct StartupCrewRenameState {
    pub(crate) index: usize,
    pub(crate) player_path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) edit: RenameEdit<PlrSelControl>,
    pub(crate) last_click: Option<Instant>,
    pub(crate) ignore_pointer_up: bool,
}

/// `C4GUI::Dialog::AdvanceFocus` order for C4StartupScenSelDlg. The option
/// strip is one recursive child here and owns its constructor-ordered icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenselDialogFocus {
    Search,
    List,
    Back,
    Definitions,
    Options,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScenselFocusSnapshot {
    pub(crate) dialog: ScenselDialogFocus,
    pub(crate) option: Option<GameOptionButton>,
}

pub(crate) const SCENSEL_SCROLLBAR_PART: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenselScrollbarTarget {
    List,
    Description,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenselScrollbarInteractionKind {
    Dragging,
    Arrow(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScenselScrollbarInteraction {
    pub(crate) target: ScenselScrollbarTarget,
    pub(crate) kind: ScenselScrollbarInteractionKind,
    pub(crate) pin: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScenselScrollbarSpec {
    pub(crate) target: ScenselScrollbarTarget,
    pub(crate) rect: clonk_frontend::classic_gui::IntRect,
    pub(crate) max_scroll: i32,
    pub(crate) offset: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MapFolderRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct MapFolderScenarioButton {
    pub(crate) entry: Option<FrontendScenario>,
    pub(crate) base_image: Option<ImageData>,
    pub(crate) overlay_image: Option<ImageData>,
    single_click: bool,
    pub(crate) area: MapFolderRect,
    pub(crate) title: String,
    pub(crate) title_font_size: i32,
    pub(crate) title_color_inactive: u32,
    pub(crate) title_color_active: u32,
    pub(crate) title_offset_x: i32,
    pub(crate) title_offset_y: i32,
    pub(crate) title_align: u8,
    pub(crate) title_use_book_font: bool,
}

impl MapFolderScenarioButton {
    /// A caption-only button, for probing how a title is rasterized.
    #[cfg(test)]
    pub(crate) fn title_probe_for_test(title: &str, font_size: i32, use_book_font: bool) -> Self {
        Self {
            entry: None,
            base_image: None,
            overlay_image: None,
            single_click: false,
            area: MapFolderRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            title: title.to_string(),
            title_font_size: font_size,
            title_color_inactive: 0x00ff_ffff,
            title_color_active: 0x00ff_ffff,
            title_offset_x: 0,
            title_offset_y: 0,
            title_align: 0,
            title_use_book_font: use_book_font,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MapFolderAccessOverlay {
    pub(crate) image: Option<ImageData>,
    pub(crate) area: MapFolderRect,
}

#[derive(Clone, Debug)]
pub(crate) struct MapFolderData {
    pub(crate) source_path: PathBuf,
    pub(crate) background: ImageData,
    pub(crate) scenario_info_area: MapFolderRect,
    pub(crate) fullscreen_background: bool,
    pub(crate) hide_title: bool,
    pub(crate) scenarios: Vec<MapFolderScenarioButton>,
    pub(crate) access_overlays: Vec<MapFolderAccessOverlay>,
    pub(crate) selected_button: Option<usize>,
}

impl MapFolderData {
    pub(crate) fn selected_entry(&self) -> Option<&FrontendScenario> {
        self.selected_button
            .and_then(|index| self.scenarios.get(index))
            .and_then(|button| button.entry.as_ref())
    }
}

#[derive(Clone, Debug)]
enum MenuLayerStyle {
    Book,
    Map(MapFolderData),
}

pub(crate) fn scensel_scrollbar_pin_travel(bar_height: i32) -> Option<i32> {
    (bar_height > 3 * SCENSEL_SCROLLBAR_PART).then_some(bar_height - 3 * SCENSEL_SCROLLBAR_PART)
}

pub(crate) fn scensel_scrollbar_pin_from_offset(
    offset: i32,
    max_scroll: i32,
    bar_height: i32,
) -> Option<i32> {
    let travel = scensel_scrollbar_pin_travel(bar_height)?;
    (max_scroll > 0).then(|| {
        let offset = offset.clamp(0, max_scroll);
        i32::try_from(i64::from(travel) * i64::from(offset) / i64::from(max_scroll))
            .unwrap_or(travel)
    })
}

pub(crate) fn scensel_scrollbar_offset_from_pin(
    pin: i32,
    max_scroll: i32,
    bar_height: i32,
) -> Option<i32> {
    let travel = scensel_scrollbar_pin_travel(bar_height)?;
    (max_scroll > 0).then(|| {
        let pin = pin.clamp(0, travel);
        i32::try_from(i64::from(max_scroll) * i64::from(pin) / i64::from(travel))
            .unwrap_or(max_scroll)
    })
}

pub(crate) fn scensel_scrollbar_jump_pin(pointer_y: i32, bar_height: i32) -> Option<i32> {
    let travel = scensel_scrollbar_pin_travel(bar_height)?;
    Some((pointer_y - SCENSEL_SCROLLBAR_PART - SCENSEL_SCROLLBAR_PART / 2).clamp(0, travel))
}

#[derive(Clone, Debug)]
pub(crate) struct MenuLayer {
    pub(crate) title: String,
    pub(crate) entries: Vec<FrontendScenario>,
    /// The folder entry this layer lists the children of (None at root);
    /// shown on the right page when nothing is selected
    /// (C4StartupScenSelDlg::UpdateSelection, cpp:1566-1572).
    folder: Option<FrontendScenario>,
    style: MenuLayerStyle,
}

pub(crate) struct MainMenuState {
    pub(crate) menu: StartupMainMenu,
    pub(crate) participants_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupView {
    MainMenu,
    ScenarioBrowser,
    NetworkLobby,
    /// C4StartupNetDlg — the network game browser ("Start Network Game").
    NetworkGame,
    Options,
    About,
    /// C4StartupPlrSelDlg — the player selection dialog.
    PlayerSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupDialog {
    MainMenu,
    ScenarioBrowser(ScenarioSelectorMode),
    NetworkGame,
    Options,
    About,
    PlayerSelection,
}

pub(crate) const STARTUP_DIALOG_FADE_STEPS: u8 = 10;
pub(crate) const INGAME_MOUSE_CAPTION_DELAY: u8 = 10;

pub(crate) struct StartupDialogFade {
    pub(crate) outgoing: Option<StartupDialog>,
    pub(crate) incoming: StartupDialog,
    /// Number of already presented fade frames. C++ advances opacity before
    /// drawing, so the first presentation changes this from zero to one.
    pub(crate) step: u8,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) underlay: Vec<u8>,
    pub(crate) outgoing_frame: Option<Vec<u8>>,
    pub(crate) outgoing_native_frame: Option<Vec<u8>>,
    pub(crate) outgoing_native_text: Vec<clonk_graphics::clonk_font::CapturedClonkText>,
    pub(crate) outgoing_native_fonts: Option<Arc<clonk_frontend::clonk_fonts::NativeClonkFontSet>>,
    pub(crate) underlay_gpu_recorder: Option<GpuSceneRecorder>,
    pub(crate) outgoing_gpu_plan: Option<NativePresentationPlan>,
}

pub(crate) struct StartupDialogFadeLayers {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) underlay: Vec<u8>,
    pub(crate) outgoing_frame: Vec<u8>,
    pub(crate) outgoing_native_frame: Vec<u8>,
    pub(crate) outgoing_native_text: Vec<clonk_graphics::clonk_font::CapturedClonkText>,
    pub(crate) underlay_gpu_recorder: Option<GpuSceneRecorder>,
    pub(crate) outgoing_gpu_plan: Option<NativePresentationPlan>,
}

#[cfg(test)]
impl StartupView {
    pub(crate) const ALL: [Self; 7] = [
        Self::MainMenu,
        Self::ScenarioBrowser,
        Self::NetworkLobby,
        Self::NetworkGame,
        Self::Options,
        Self::About,
        Self::PlayerSelection,
    ];
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IngameMouseState {
    pub(crate) start: ViewportPointer,
    pub(crate) last: ViewportPointer,
    /// Last endpoint at which `C4Player::FoWIsVisible` allowed DragSelect to
    /// rebuild the local Selection. The pointer/frame keeps moving through
    /// fog; the actual ordered membership is cached in `ingame_dragged_objects`.
    pub(crate) selection_last: ViewportPointer,
    pub(crate) moved: bool,
    /// The stored DownCursor was Crosshair/Dig, so crossing the drag
    /// threshold enters C4MC_Drag_Selecting rather than moving an object.
    pub(crate) selection_frame: bool,
    /// Once a drag first finds crew or carryables, C++ keeps that selection
    /// type for the rest of the frame's lifetime.
    pub(crate) selection_kind: IngameDragSelectionKind,
    /// Value-copied `DownRegion`: its command, data, and target remain the
    /// down-time payload even if the HUD changes before button-up.
    pub(crate) down_region: Option<IngameViewportRegion>,
    /// `UpdateTargetRegion` cancels an active landscape selection frame and
    /// resets DownCursor so it cannot restart while the button stays down.
    pub(crate) selection_cancelled_by_region: bool,
    /// Region drag eligibility is fixed when the pointer first crosses the
    /// drag threshold, matching C4MouseControl::DragNone.
    pub(crate) region_drag_started: bool,
    /// World carryable/vehicle eligibility is likewise latched at the first
    /// threshold crossing so opening a GUI cannot manufacture a moving drag.
    pub(crate) world_drag_started: bool,
    /// DragMoving refreshes this fully resolved cursor on later pointer
    /// updates; button up consumes it without re-running DragMoving.
    pub(crate) region_drag_cursor: Option<IngameRegionDragCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IngameMouseHelpCaption {
    pub(crate) text: String,
    pub(crate) keep_moves: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngameMouseCursorKind {
    Region,
    Help,
    Crosshair,
    Dig,
    DigMaterial,
    Enter,
    Grab,
    Ungrab,
    Carryable,
    DigObject,
    Chop,
    Build,
    Select,
    Attack,
    JumpLeft,
    JumpRight,
    Scrolling(clonk_frontend::MouseCursorPhase),
    Drop,
    ThrowLeft(Vector2),
    ThrowRight(Vector2),
    Put,
    Vehicle,
    VehiclePut,
    Construct,
    Nothing,
}

impl IngameMouseCursorKind {
    pub(crate) fn phase(self) -> MouseCursorPhase {
        match self {
            Self::Region => MouseCursorPhase::Region,
            Self::Help => MouseCursorPhase::Help,
            Self::Crosshair => MouseCursorPhase::Crosshair,
            Self::Dig => MouseCursorPhase::Dig,
            Self::DigMaterial => MouseCursorPhase::DigMaterial,
            Self::Enter => MouseCursorPhase::Enter,
            Self::Grab => MouseCursorPhase::Grab,
            Self::Ungrab => MouseCursorPhase::Ungrab,
            Self::Carryable => MouseCursorPhase::Object,
            Self::DigObject => MouseCursorPhase::DigObject,
            Self::Chop => MouseCursorPhase::Chop,
            Self::Build => MouseCursorPhase::Build,
            Self::Select => MouseCursorPhase::Select,
            Self::Attack => MouseCursorPhase::Attack,
            Self::JumpLeft => MouseCursorPhase::JumpLeft,
            Self::JumpRight => MouseCursorPhase::JumpRight,
            Self::Scrolling(phase) => phase,
            Self::Drop => MouseCursorPhase::Drop,
            Self::ThrowLeft(_) => MouseCursorPhase::ThrowLeft,
            Self::ThrowRight(_) => MouseCursorPhase::ThrowRight,
            Self::Put => MouseCursorPhase::Put,
            Self::Vehicle => MouseCursorPhase::Vehicle,
            Self::VehiclePut => MouseCursorPhase::VehiclePut,
            Self::Construct => MouseCursorPhase::Construct,
            Self::Nothing => MouseCursorPhase::Nothing,
        }
    }

    pub(crate) fn throw_landing(self) -> Option<Vector2> {
        match self {
            Self::ThrowLeft(landing) | Self::ThrowRight(landing) => Some(landing),
            _ => None,
        }
    }

    pub(crate) fn allows_add_marker(self) -> bool {
        !matches!(
            self,
            Self::Region | Self::Select | Self::JumpLeft | Self::JumpRight
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IngameMouseCaption {
    pub(crate) text: String,
    pub(crate) viewport_index: usize,
    pub(crate) position: Vector2,
    pub(crate) caption_bottom_y: Option<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct IngameMouseCaptionState {
    pub(crate) cursor: IngameMouseCursorKind,
    pub(crate) time_on_target: u8,
    pub(crate) keep_caption: usize,
    pub(crate) caption: Option<IngameMouseCaption>,
}

impl Default for IngameMouseCaptionState {
    fn default() -> Self {
        Self {
            cursor: IngameMouseCursorKind::Region,
            time_on_target: 0,
            keep_caption: 0,
            caption: None,
        }
    }
}

impl IngameMouseCaptionState {
    pub(crate) fn begin_move(&mut self) {
        if self.keep_caption != 0 {
            self.keep_caption -= 1;
        } else {
            self.caption = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngameDragSelectionKind {
    Unknown,
    Crew,
    Objects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngameViewportRegion {
    ViewportButton(clonk_frontend::hud::ViewportButton),
    Command(u8),
    Inventory(ObjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngameRegionDragCursor {
    Drop,
    Throw,
    Put(ObjectId),
    Vehicle,
    VehiclePut(ObjectId),
}

impl IngameViewportRegion {
    pub(crate) fn control(self) -> (u8, i32) {
        match self {
            Self::ViewportButton(button) => (button.command(), 0),
            Self::Command(command) => (command, 0),
            // COM_Contents (C4Constants.h:188); Data is the target number.
            Self::Inventory(target) => (9, target.as_u64() as i32),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IngameButtonMouseState {
    pub(crate) motion: IngameMouseState,
    /// The copied C4MouseControl::DownTarget. Moving-drag classification must
    /// use the cursor at button-down, not whichever object is below release.
    pub(crate) down_target: Option<ObjectId>,
    /// The down target came from a viewport region rather than the world pick
    /// layer. Region drags use cached Carryable/Grab state; only the physical
    /// right button expands same-ID inventory groups.
    pub(crate) down_region: bool,
    /// Fog replaced the native DownCursor with Nothing. Moving into a visible
    /// area later must remain DragNone and use the release-time cursor.
    pub(crate) down_cursor_nothing: bool,
    /// The copied DownCursor was C4MC_Cursor_Help. Crossing the drag
    /// threshold must remain DragNone and button-up must emit no command.
    pub(crate) down_cursor_help: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct LobbyParticipantState {
    pub(crate) name: String,
    pub(crate) ready: bool,
    pub(crate) kind: ParticipantKind,
}

impl LobbyParticipantState {
    fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        Self {
            name: name.into(),
            ready: false,
            kind,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LobbyPointerRegion {
    Menu,
    Panel,
}

#[derive(Clone, Debug)]
pub(crate) struct NetworkLobbyLayout {
    pub(crate) exit_button: GuiRect,
    pub(crate) ready_button: GuiRect,
    pub(crate) start_button: Option<GuiRect>,
    preload_button: Option<GuiRect>,
    pub(crate) sheet_buttons: Vec<(LobbySheet, GuiRect)>,
    roster_client: GuiRect,
    pub(crate) resource_save_buttons: Vec<(i32, GuiRect)>,
    pub(crate) external_chat_button: Option<GuiRect>,
    menu_region_max_x: f32,
}

impl NetworkLobbyLayout {
    fn from_classic(
        layout: &LobbyLayout,
        active_sheet: LobbySheet,
        resource_rows: &BTreeMap<i32, LobbyResourceRow>,
        resource_scroll: i32,
        text_line_height: i32,
    ) -> Self {
        let as_gui_rect = |rect: clonk_frontend::classic_gui::IntRect| {
            GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
        };
        let sheet_buttons = layout
            .tab_buttons
            .iter()
            .filter_map(|button| button.sheet.map(|sheet| (sheet, as_gui_rect(button.rect))))
            .collect();
        let external_chat_button = layout
            .tab_buttons
            .iter()
            .find(|button| button.control == LobbyControl::ChatDialog)
            .map(|button| as_gui_rect(button.rect));
        let row_height = text_line_height.max(1).saturating_add(4);
        let resource_save_buttons = if active_sheet == LobbySheet::Resources {
            resource_rows
                .values()
                .enumerate()
                .filter(|(_, row)| row.save_possible)
                .filter_map(|(index, row)| {
                    let y = layout.roster_client.y
                        + i32::try_from(index)
                            .unwrap_or(i32::MAX)
                            .saturating_mul(row_height)
                        - resource_scroll
                        + 1;
                    let rect = clonk_frontend::classic_gui::IntRect::new(
                        layout.roster_client.x + layout.roster_client.w - 18,
                        y,
                        16,
                        16,
                    );
                    let visible = rect.x < layout.roster_client.x + layout.roster_client.w
                        && rect.x + rect.w > layout.roster_client.x
                        && rect.y < layout.roster_client.y + layout.roster_client.h
                        && rect.y + rect.h > layout.roster_client.y;
                    visible.then(|| (row.id, as_gui_rect(rect)))
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            exit_button: as_gui_rect(layout.exit_button),
            ready_button: as_gui_rect(layout.ready_checkbox),
            start_button: layout.run_button.map(as_gui_rect),
            preload_button: layout.preload_button.map(as_gui_rect),
            sheet_buttons,
            roster_client: as_gui_rect(layout.roster_client),
            resource_save_buttons,
            external_chat_button,
            // C4GameLobby has no startup scenario-selector pane. Keep all
            // ordinary pointer traffic on the fullscreen dialog itself.
            menu_region_max_x: -1.0,
        }
    }
}

/// A restart announced by the host, and the window in which this client keeps
/// trying to find it again.
///
/// The host re-binds the port it was already configured with and keeps its
/// password (`C4Application::QuitGame` backs the password up across
/// `Game.Clear` and hands the same scenario to a fresh `Game::Init`,
/// src/C4Application.cpp:373-405), so the *whole join this client already made*
/// is what to repeat — not just its address. Rebuilding the settings from
/// config would drop the password, the netpuncher brokerage and every route
/// but the first, which is exactly what a password-protected or NAT'd host
/// needs. Attempts are spaced because the host is mid-teardown when the notice
/// arrives and cannot accept a connection yet.
#[derive(Clone, Debug)]
pub(crate) struct PendingHostRejoin {
    pub(crate) settings: ClientSettings,
    pub(crate) deadline: Instant,
    pub(crate) next_attempt_at: Option<Instant>,
}

/// What a round teardown does with the live network session.
///
/// `C4Game::Clear` knows only [`Self::Clear`]: the session dies with the round
/// it belongs to (src/C4Game.cpp:544-582). [`Self::Retain`] is the port-only
/// case behind `clonk_network::host_restart` — the host restarts the round
/// without closing the session, so the manager, its sockets and every client
/// connection outlive the round and carry straight on into the next lobby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkSessionTeardown {
    Clear,
    Retain,
}

/// Spacing between reconnect attempts while the host is re-hosting. The first
/// attempt necessarily races the host's own teardown, so a refused connection
/// is the expected case rather than a failure.
pub(crate) const HOST_REJOIN_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Longest reconnect window this client will honour, whatever a peer asks for.
/// A restart that has not come back in two minutes is not coming back, and the
/// number arrives from the network.
pub(crate) const MAX_HOST_RESTART_REJOIN_SECONDS: u16 = 120;

#[derive(Clone, Debug)]
pub(crate) struct NetworkLobbyState {
    /// The native client constructs one C4GameLobby::MainDlg for the whole
    /// lobby lifetime. Keep the Rust controller equally persistent so roster
    /// selection/focus/scroll, edit capture and TextWindow scroll state all
    /// survive rendering projections and authoritative refreshes.
    pub(crate) controller: ClassicGameLobby,
    pub(crate) participants: BTreeMap<ClientId, LobbyParticipantState>,
    /// Authoritative PlayerInfo projection shared with the host's persistent
    /// controller so peer lobbies converge after the same direct control.
    pub(crate) roster_rows: Vec<LobbyRosterRow>,
    /// Distinguishes the initial participant-only fallback from an
    /// authoritative projection that is legitimately empty.
    pub(crate) roster_rows_authoritative: bool,
    client_telemetry: clonk_network::RuntimeLobbyClientTelemetry,
    pub(crate) active_players: i32,
    pub(crate) max_players: i32,
    pub(crate) has_teams: bool,
    pub(crate) league_mode: bool,
    /// Construction-time snapshot of C4ChatDlg::IsChatActive(). The native
    /// lobby does not add or remove this button as IRC state later changes.
    has_external_chat: bool,
    pub(crate) active_sheet: LobbySheet,
    /// Client-side C4Network2ResDlg snapshot. Network updates continue while
    /// hidden; the reconstructed controller receives it only when active.
    pub(crate) resource_rows: BTreeMap<i32, LobbyResourceRow>,
    pub(crate) resource_scroll: i32,
    pub(crate) scenario_description: LobbyScenarioDescriptionState,
    scenario_scroll: i32,
    pub(crate) resources_loaded: bool,
    pub(crate) preload: LobbyPreloadState,
    pub(crate) labels: LobbyLabels,
    pub(crate) logs: Vec<LobbyLogLine>,
    pub(crate) chat_edit: LobbyChatEditView,
    pub(crate) chat_history_index: i32,
    /// Active one-second `/sound` marker per sender. `true` is the muted icon.
    client_sound_status: HashMap<ClientId, (bool, Instant)>,
    pub(crate) local_client_id: ClientId,
    pub(crate) is_host: bool,
    selected_identifier: Option<String>,
    selected_title: Option<String>,
    /// Raw C++ countdown timer. `None` is the distinguished abort packet;
    /// `Some(0)` is the final start transition.
    pub(crate) countdown: Option<i32>,
    pub(crate) layout: Option<NetworkLobbyLayout>,
    pub(crate) pointer: Option<GuiPoint>,
    pub(crate) last_roster_click: Option<(LobbyRosterId, Instant)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LobbyAction {
    ExitRequested,
    ToggleReady,
    StartGame,
    Preload,
    SelectSheet(LobbySheet),
    SaveResource(i32),
    OpenExternalIrcChat,
    SubmitMessage(String),
    ChatEdited,
}

pub(crate) const DEFAULT_LOBBY_COUNTDOWN_SECONDS: i32 = 5;
pub(crate) const ALMOST_START_LOBBY_COUNTDOWN_SECONDS: i32 = 10;

/// `C4PacketCountdown::GetCountdownMsg` (src/C4GameLobby.cpp:50-60): the last
/// seconds are a bare `"n..."`, and everything else — including the very first
/// message, whatever its value — is the full `IDS_PRC_COUNTDOWN` sentence.
///
/// `template` is the resolved resource with its `%d` already replaced by
/// `{seconds}`, as `classic_lobby_labels` prepares it.
pub(crate) fn lobby_countdown_message(seconds: i32, initial: bool, template: &str) -> String {
    if seconds < ALMOST_START_LOBBY_COUNTDOWN_SECONDS && !initial {
        format!("{seconds}...")
    } else {
        template.replace("{seconds}", &seconds.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MessageControlOutcome {
    pub(crate) rejected: bool,
    pub(crate) displayed: bool,
    pub(crate) say_displayed: bool,
    pub(crate) sound_attempted: bool,
    pub(crate) sound_played: bool,
    pub(crate) lobby_sound: bool,
    pub(crate) attention_requested: bool,
}

pub(crate) const CONTROL_LOG_COLOR: u32 = 0x00af_afaf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostLobbyCountdown {
    pub(crate) remaining: i32,
}

impl HostLobbyCountdown {
    pub(crate) fn new() -> Self {
        Self::with_seconds(DEFAULT_LOBBY_COUNTDOWN_SECONDS)
    }

    pub(crate) fn with_seconds(seconds: i32) -> Self {
        Self {
            remaining: seconds.max(0),
        }
    }

    pub(crate) fn advance(&mut self) -> (i32, bool) {
        self.remaining = (self.remaining - 1).max(0);
        let broadcast = self.remaining <= ALMOST_START_LOBBY_COUNTDOWN_SECONDS
            || (self.remaining <= 600 && self.remaining % 10 == 0)
            || self.remaining % 60 == 0;
        (self.remaining, broadcast)
    }
}

pub(crate) const DEFAULT_LOBBY_READY_CHECK_COOLDOWN_SECONDS: i64 = 10;
const MINIMUM_LOBBY_READY_CHECK_COOLDOWN_SECONDS: i64 = 5;
pub(crate) const LOBBY_READY_CHECK_PROMPT_SECONDS: u32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LeagueVoteSubject {
    pub(crate) vote_type: u8,
    pub(crate) data: i32,
}

const LEAGUE_VOTE_TIMEOUT_SECONDS: i64 = 10;
const LEAGUE_VOTE_MIN_INTERVAL_SECONDS: i64 = 120;

impl From<clonk_engine::VoteControlData> for LeagueVoteSubject {
    fn from(vote: clonk_engine::VoteControlData) -> Self {
        Self {
            vote_type: vote.vote_type,
            data: vote.data,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct LeagueVoteState {
    pub(crate) ballots: Vec<clonk_engine::VoteControlData>,
    pub(crate) paused_for_vote: bool,
    started_at_seconds: Option<i64>,
    last_own_vote_at_seconds: Option<i64>,
}

impl LeagueVoteState {
    pub(crate) fn add(&mut self, vote: clonk_engine::VoteControlData) {
        let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
        self.add_at(vote, now);
    }

    pub(crate) fn add_at(&mut self, vote: clonk_engine::VoteControlData, now: i64) {
        if self.ballots.is_empty() {
            self.started_at_seconds = Some(now);
        }
        self.ballots.push(vote);
    }

    pub(crate) fn take_timed_out_subject_at(&mut self, now: i64) -> Option<LeagueVoteSubject> {
        let subject = self
            .started_at_seconds
            .zip(self.ballots.first())
            .filter(|(started_at, _)| now > started_at.saturating_add(LEAGUE_VOTE_TIMEOUT_SECONDS))
            .map(|(_, vote)| LeagueVoteSubject::from(*vote));
        if subject.is_some() {
            self.started_at_seconds = Some(now);
        }
        subject
    }

    pub(crate) fn try_submit_own_vote_at(&mut self, subject: LeagueVoteSubject, now: i64) -> bool {
        if self.subject_active(subject) {
            return true;
        }
        if self.last_own_vote_at_seconds.is_some_and(|last_vote| {
            now < last_vote.saturating_add(LEAGUE_VOTE_MIN_INTERVAL_SECONDS)
        }) {
            return false;
        }
        self.last_own_vote_at_seconds = Some(now);
        true
    }

    pub(crate) fn first_ballot(&self, client_id: i32, subject: LeagueVoteSubject) -> Option<bool> {
        self.ballots
            .iter()
            .find(|vote| vote.by_client == client_id && LeagueVoteSubject::from(**vote) == subject)
            .map(|vote| vote.approve)
    }

    pub(crate) fn end(
        &mut self,
        subject: LeagueVoteSubject,
        approve: bool,
        local_client_id: Option<i32>,
    ) -> Option<i32> {
        let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
        self.end_at(subject, approve, local_client_id, now)
    }

    pub(crate) fn end_at(
        &mut self,
        subject: LeagueVoteSubject,
        approve: bool,
        local_client_id: Option<i32>,
        now: i64,
    ) -> Option<i32> {
        let origin = self
            .ballots
            .iter()
            .find(|vote| LeagueVoteSubject::from(**vote) == subject)
            .map(|vote| vote.by_client);
        self.ballots
            .retain(|vote| LeagueVoteSubject::from(*vote) != subject);
        self.started_at_seconds = Some(now);
        if approve
            && origin
                .zip(local_client_id)
                .is_some_and(|(origin, local_client)| origin == local_client)
        {
            self.last_own_vote_at_seconds = None;
        }
        origin
    }

    pub(crate) fn subject_active(&self, subject: LeagueVoteSubject) -> bool {
        self.ballots
            .iter()
            .any(|vote| LeagueVoteSubject::from(*vote) == subject)
    }

    pub(crate) fn first_subject_needing_vote(
        &self,
        local_client_id: i32,
    ) -> Option<clonk_engine::VoteControlData> {
        self.ballots.iter().copied().find(|vote| {
            self.first_ballot(local_client_id, LeagueVoteSubject::from(*vote))
                .is_none()
        })
    }

    pub(crate) fn clear(&mut self) {
        self.ballots.clear();
        self.paused_for_vote = false;
        self.started_at_seconds = None;
        self.last_own_vote_at_seconds = None;
    }
}

pub(crate) fn lobby_ready_check_message(remaining_seconds: u32) -> String {
    format!("The host wants to know whether you're ready.|{remaining_seconds} seconds remaining.")
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LobbyReadyCheckCooldown {
    pub(crate) duration: Duration,
    last_reset: Option<Instant>,
}

impl Default for LobbyReadyCheckCooldown {
    fn default() -> Self {
        Self::from_config_seconds(DEFAULT_LOBBY_READY_CHECK_COOLDOWN_SECONDS)
    }
}

impl LobbyReadyCheckCooldown {
    pub(crate) fn from_config_seconds(seconds: i64) -> Self {
        let seconds = seconds.max(MINIMUM_LOBBY_READY_CHECK_COOLDOWN_SECONDS);
        Self {
            duration: Duration::from_secs(u64::try_from(seconds).unwrap_or(5)),
            last_reset: None,
        }
    }

    pub(crate) fn try_reset_at(&mut self, now: Instant) -> bool {
        let elapsed = self
            .last_reset
            .and_then(|last_reset| now.checked_duration_since(last_reset))
            .is_none_or(|elapsed| elapsed >= self.duration);
        if elapsed {
            self.last_reset = Some(now);
        }
        elapsed
    }

    pub(crate) fn remaining_seconds_at(&self, now: Instant) -> u64 {
        self.last_reset
            .and_then(|last_reset| now.checked_duration_since(last_reset))
            .filter(|elapsed| *elapsed < self.duration)
            .map(|elapsed| (self.duration - elapsed).as_secs())
            .unwrap_or(0)
    }
}

impl NetworkLobbyState {
    pub(crate) fn new(local_client_id: ClientId, local_name: String, is_host: bool) -> Self {
        let mut participants = BTreeMap::new();
        participants.insert(
            local_client_id,
            LobbyParticipantState::new(local_name, ParticipantKind::Player),
        );
        if !is_host && local_client_id != 0 {
            participants
                .entry(0)
                .or_insert_with(|| LobbyParticipantState::new("Host", ParticipantKind::Player));
        }
        let controller = ClassicGameLobby::new(
            if is_host {
                LobbyRole::Host
            } else {
                LobbyRole::Client
            },
            String::new(),
            0,
            1,
            false,
            false,
            false,
            false,
            DEFAULT_LOBBY_COUNTDOWN_SECONDS,
            Vec::new(),
        );
        Self {
            controller,
            participants,
            roster_rows: Vec::new(),
            roster_rows_authoritative: false,
            client_telemetry: clonk_network::RuntimeLobbyClientTelemetry::default(),
            active_players: 0,
            max_players: 1,
            has_teams: false,
            league_mode: false,
            has_external_chat: false,
            active_sheet: LobbySheet::Players,
            resource_rows: BTreeMap::new(),
            resource_scroll: 0,
            scenario_description: LobbyScenarioDescriptionState::default(),
            scenario_scroll: 0,
            resources_loaded: false,
            preload: LobbyPreloadState::new(false),
            labels: LobbyLabels::default(),
            logs: Vec::new(),
            chat_edit: LobbyChatEditView::default(),
            chat_history_index: -1,
            client_sound_status: HashMap::new(),
            local_client_id,
            is_host,
            selected_identifier: None,
            selected_title: None,
            countdown: None,
            layout: None,
            pointer: None,
            last_roster_click: None,
        }
    }

    pub(crate) fn with_external_chat(mut self, has_external_chat: bool) -> Self {
        self.has_external_chat = has_external_chat;
        self.controller.set_has_external_chat(has_external_chat);
        self
    }

    pub(crate) fn with_preloading(mut self, automatic: bool, labels: LobbyLabels) -> Self {
        self.preload = LobbyPreloadState::new(automatic);
        self.controller.set_labels(labels.clone());
        self.labels = labels;
        self
    }

    pub(crate) fn scenario_label(&self) -> String {
        self.selected_title
            .clone()
            .unwrap_or_else(|| "Select a scenario from the list".to_string())
    }

    pub(crate) fn selected_identifier(&self) -> Option<&str> {
        self.selected_identifier.as_deref()
    }

    pub(crate) fn select_scenario(&mut self, identifier: &str, title: &str) {
        self.selected_identifier = Some(identifier.to_string());
        self.selected_title = Some(title.to_string());
    }

    pub(crate) fn set_scenario_title(&mut self, title: &str) {
        self.selected_title = (!title.is_empty()).then(|| title.to_string());
        self.controller.set_scenario_title(title);
    }

    pub(crate) fn update_layout(&mut self, width: f32, height: f32) -> &NetworkLobbyLayout {
        let role = if self.is_host {
            LobbyRole::Host
        } else {
            LobbyRole::Client
        };
        let mut layout = clonk_frontend::game_lobby::game_lobby_layout(
            width as i32,
            height as i32,
            34,
            22,
            role,
            self.has_teams,
            self.has_external_chat,
        );
        if self.active_sheet == LobbySheet::Resources
            && self.preload.manual_button_present
            && self.preload.eligible
        {
            layout.preload_button = Some(clonk_frontend::classic_gui::IntRect::new(
                layout.roster.x,
                layout.roster.y + (layout.roster.h - 32).max(0),
                layout.roster.w,
                32.min(layout.roster.h),
            ));
        }
        self.layout = Some(NetworkLobbyLayout::from_classic(
            &layout,
            self.active_sheet,
            &self.resource_rows,
            self.resource_scroll,
            22,
        ));
        self.layout.as_ref().expect("layout just initialised")
    }

    pub(crate) fn pointer_region(&self, point: GuiPoint) -> LobbyPointerRegion {
        if let Some(layout) = self.layout.as_ref() {
            if point.x <= layout.menu_region_max_x {
                LobbyPointerRegion::Menu
            } else {
                LobbyPointerRegion::Panel
            }
        } else {
            LobbyPointerRegion::Menu
        }
    }

    pub(crate) fn handle_panel_pointer_move(&mut self, point: GuiPoint) {
        self.pointer = Some(point);
    }

    pub(crate) fn pointer_left(&mut self) {
        self.pointer = None;
        self.last_roster_click = None;
        self.controller.pointer_left();
    }

    pub(crate) fn exit_hotkey(&self) -> Option<char> {
        expand_hotkey_markup(&self.labels.exit).1
    }

    pub(crate) fn register_peer(
        &mut self,
        client_id: ClientId,
        name: String,
        kind: ParticipantKind,
    ) {
        let ready = self
            .participants
            .get(&client_id)
            .map(|participant| participant.ready)
            .unwrap_or(false);
        self.participants
            .insert(client_id, LobbyParticipantState { name, ready, kind });
    }

    pub(crate) fn replace_participants_from_clients(
        &mut self,
        clients: &[clonk_engine::ClientCoreControlData],
    ) {
        self.participants = clients
            .iter()
            .filter_map(|client| {
                ClientId::try_from(client.client_id).ok().map(|client_id| {
                    (
                        client_id,
                        LobbyParticipantState {
                            name: legacy_presentation_text(client.name.as_bytes()),
                            ready: client.lobby_ready,
                            kind: if client.observer {
                                ParticipantKind::Observer
                            } else {
                                ParticipantKind::Player
                            },
                        },
                    )
                })
            })
            .collect();
    }

    pub(crate) fn unregister_peer(&mut self, client_id: ClientId) {
        if client_id == self.local_client_id {
            return;
        }
        self.participants.remove(&client_id);
        self.client_sound_status.remove(&client_id);
        if !self.is_host && client_id == 0 {
            self.participants
                .entry(0)
                .or_insert_with(|| LobbyParticipantState::new("Host", ParticipantKind::Player));
        }
    }

    pub(crate) fn toggle_local_ready(&mut self) -> bool {
        if let Some(participant) = self.participants.get_mut(&self.local_client_id) {
            participant.ready = !participant.ready;
            participant.ready
        } else {
            false
        }
    }

    pub(crate) fn apply_ready_check(
        &mut self,
        packet: clonk_network::ReadyCheckPacket,
    ) -> Option<ClientId> {
        if packet.data.vote_requested() {
            return None;
        }
        let Ok(client_id) = ClientId::try_from(packet.client_id) else {
            return None;
        };
        let participant = self.participants.get_mut(&client_id)?;
        let ready = packet.data.is_ready();
        if participant.ready == ready {
            return None;
        }
        participant.ready = ready;
        Some(client_id)
    }

    pub(crate) fn apply_lobby_countdown(&mut self, packet: clonk_network::LobbyCountdownPacket) {
        self.countdown = (!packet.is_abort()).then_some(packet.countdown());
    }

    pub(crate) fn local_ready(&self) -> bool {
        self.participants
            .get(&self.local_client_id)
            .map(|participant| participant.ready)
            .unwrap_or(false)
    }

    pub(crate) fn push_log(&mut self, line: LobbyLogLine) {
        self.logs.push(line.clone());
        self.controller.push_log(line);
    }

    pub(crate) fn note_client_sound(&mut self, client_id: i32, muted: bool) {
        if let Ok(client_id) = ClientId::try_from(client_id) {
            self.client_sound_status
                .insert(client_id, (muted, Instant::now()));
        }
        self.controller.note_client_sound(client_id, muted);
    }

    pub(crate) fn set_client_telemetry(
        &mut self,
        telemetry: clonk_network::RuntimeLobbyClientTelemetry,
    ) -> bool {
        let mut changed = self.client_telemetry != telemetry;
        self.client_telemetry = telemetry;
        changed |= apply_classic_lobby_client_telemetry(
            &mut self.roster_rows,
            self.local_client_id,
            &self.client_telemetry,
        );
        changed
    }

    fn participant_roster_rows(&self) -> Vec<LobbyRosterRow> {
        let mut rows = self
            .participants
            .iter()
            .map(|(client_id, participant)| {
                LobbyRosterRow::Client(LobbyClientRow {
                    id: i32::try_from(*client_id).unwrap_or(i32::MAX),
                    name: participant.name.clone(),
                    nick: String::new(),
                    color: [255, 255, 255, 255],
                    status: if let Some((muted, _)) = self.client_sound_status.get(client_id) {
                        if *muted {
                            LobbyClientStatus::MutedSound
                        } else {
                            LobbyClientStatus::Sound
                        }
                    } else if *client_id == 0 {
                        LobbyClientStatus::Host
                    } else if matches!(participant.kind, ParticipantKind::Observer) {
                        LobbyClientStatus::Observer
                    } else if participant.ready {
                        LobbyClientStatus::Ready
                    } else {
                        LobbyClientStatus::Client
                    },
                    local: *client_id == self.local_client_id,
                    connected: *client_id != self.local_client_id,
                    resource_progress: None,
                    ping_ms: None,
                })
            })
            .collect::<Vec<_>>();
        apply_classic_lobby_client_telemetry(
            &mut rows,
            self.local_client_id,
            &self.client_telemetry,
        );
        rows
    }

    pub(crate) fn visible_roster_rows(&self) -> Vec<LobbyRosterRow> {
        if self.roster_rows_authoritative {
            self.roster_rows.clone()
        } else {
            self.participant_roster_rows()
        }
    }

    pub(crate) fn visible_client_is_local(&self, client_id: i32) -> Option<bool> {
        self.visible_roster_rows()
            .into_iter()
            .find_map(|row| match row {
                LobbyRosterRow::Client(client) if client.id == client_id => Some(client.local),
                _ => None,
            })
    }

    pub(crate) fn sync_classic_controller(&mut self) {
        let now = Instant::now();
        self.client_sound_status.retain(|_, (_, started)| {
            now.checked_duration_since(*started)
                .is_none_or(|elapsed| elapsed < Duration::from_secs(1))
        });
        let rows = self.visible_roster_rows();
        let local_ready = self.local_ready();
        if !self.has_teams && self.active_sheet == LobbySheet::Teams {
            self.active_sheet = LobbySheet::Players;
        }
        self.controller
            .set_scenario_title(self.selected_title.as_deref().unwrap_or_default());
        self.controller.set_has_teams(self.has_teams);
        self.controller.set_league_mode(self.league_mode);
        self.controller
            .set_has_external_chat(self.has_external_chat);
        if self.controller.rows() != rows {
            self.controller.set_rows(rows);
        }
        self.controller
            .set_player_count(self.active_players, self.max_players.max(1));
        if self.controller.resources_loaded() != self.resources_loaded {
            let _ = self.controller.set_resources_loaded(self.resources_loaded);
        }
        self.controller.set_ready(local_ready);
        if self.controller.labels() != &self.labels {
            self.controller.set_labels(self.labels.clone());
        }
        self.controller
            .set_preload_button_state(self.preload.manual_button_present, self.preload.eligible);
        self.controller.set_active_sheet(self.active_sheet);
        if self.controller.scenario_text() != &self.scenario_description.text {
            self.controller
                .set_scenario_text(self.scenario_description.text.clone());
        }
        self.controller.set_scenario_scroll(self.scenario_scroll);
        let resources = self.resource_rows.values().cloned().collect::<Vec<_>>();
        if self.controller.resource_rows() != resources {
            self.controller.set_resource_rows(resources);
        }
        self.controller.set_resource_scroll(self.resource_scroll);
        if self.controller.logs() != self.logs {
            self.controller.set_logs(self.logs.clone());
        }
        if self.controller.chat_edit_view() != &self.chat_edit {
            self.controller.set_chat_edit_view(self.chat_edit.clone());
        }
    }

    /// Reads interaction-owned state back out of the retained controller
    /// after it has processed input, so the adapter's authoritative fields
    /// stay coherent with what the one live MainDlg now shows.
    fn retain_controller_state(&mut self) {
        self.resource_scroll = self.controller.resource_scroll();
        self.scenario_scroll = self.controller.scenario_scroll();
        self.chat_edit = self.controller.chat_edit_view().clone();
    }

    pub(crate) fn synchronize_classic_controller(
        &mut self,
        width: i32,
        height: i32,
        fonts: &clonk_frontend::ClonkFontSet,
        scenario_game_options: &GameOptionButtons,
    ) -> (LobbyLayout, LobbyRosterLayout, GameOptionButtons) {
        self.sync_classic_controller();
        // C4GameLobby owns one C4GameOptionButtons instance for the dialog
        // lifetime. Render a projection of the retained application strip so
        // hover, press, focus, and tooltip timing survive frame boundaries.
        let mut options = scenario_game_options.clone();
        options.set_lobby_league(self.league_mode);
        options.set_countdown(self.controller.countdown().is_locked());
        let layout = self.controller.layout(width, height, fonts);
        options.set_bounds(layout.game_option_strip);
        let _ = self.controller.chat_scroll_metrics(&layout, &fonts.text);
        let roster = self.controller.right_list_layout(&layout, fonts);
        self.layout = Some(NetworkLobbyLayout::from_classic(
            &layout,
            self.active_sheet,
            &self.resource_rows,
            self.resource_scroll,
            fonts.text.line_height,
        ));
        (layout, roster, options)
    }

    #[cfg(test)]
    pub(crate) fn classic_render_state(
        &mut self,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<(ClassicGameLobby, GameOptionButtons)> {
        let fonts = assets
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful lobby fonts are unavailable")?;
        let (_, _, options) = self.synchronize_classic_controller(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            scenario_game_options,
        );
        Ok((self.controller.clone(), options))
    }

    pub(crate) fn with_classic_controller_input<T>(
        &mut self,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
        input: impl FnOnce(&mut ClassicGameLobby, &LobbyLayout, &LobbyRosterLayout) -> T,
    ) -> Result<T> {
        let fonts = assets
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful lobby fonts are unavailable")?;
        let (layout, roster, _) = self.synchronize_classic_controller(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            scenario_game_options,
        );
        let result = input(&mut self.controller, &layout, &roster);
        self.retain_controller_state();
        Ok(result)
    }

    pub(crate) fn classic_pointer_move(
        &mut self,
        point: GuiPoint,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.pointer = Some(point);
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| controller.pointer_move(point, layout, roster),
        )
    }

    pub(crate) fn classic_pointer_down(
        &mut self,
        point: GuiPoint,
        double_click: bool,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| {
                if double_click {
                    controller.pointer_double_click(point, layout, roster)
                } else {
                    controller.pointer_down(point, layout, roster)
                }
            },
        )
    }

    pub(crate) fn classic_note_pointer_button(
        &mut self,
        point: GuiPoint,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<()> {
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| {
                controller.note_pointer_button(point, layout, roster);
            },
        )
    }

    pub(crate) fn classic_secondary_down(
        &mut self,
        point: GuiPoint,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| controller.pointer_secondary_down(point, layout, roster),
        )
    }

    pub(crate) fn classic_context_key(
        &mut self,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| {
                let actions = controller.chat_context_from_key(layout);
                if !actions.is_empty() {
                    return actions;
                }
                let anchor = controller
                    .selected_roster_id()
                    .and_then(|selected| {
                        roster.rows.iter().find(|row_layout| {
                            controller
                                .rows()
                                .get(row_layout.index)
                                .is_some_and(|row| &row.id() == selected)
                        })
                    })
                    .map(|row| {
                        GuiPoint::new(
                            (row.rect.x + row.rect.w / 2) as f32,
                            (row.rect.y + row.rect.h / 2) as f32,
                        )
                    })
                    .unwrap_or_else(|| {
                        GuiPoint::new(
                            (layout.roster.x + layout.roster.w / 2) as f32,
                            (layout.roster.y + layout.roster.h / 2) as f32,
                        )
                    });
                controller.request_focused_context(anchor)
            },
        )
    }

    pub(crate) fn classic_hotkey(&mut self, hotkey: char) -> Vec<ClassicLobbyAction> {
        self.sync_classic_controller();
        self.controller.hotkey(hotkey, Instant::now())
    }

    pub(crate) fn classic_middle_down(
        &mut self,
        point: GuiPoint,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| controller.pointer_middle_down(point, layout, roster),
        )
    }

    pub(crate) fn classic_touch(
        &mut self,
        phase: TouchPhase,
        point: GuiPoint,
        double_click: bool,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<Vec<ClassicLobbyAction>> {
        self.pointer = (!matches!(phase, TouchPhase::Cancelled)).then_some(point);
        self.with_classic_controller_input(
            surface,
            assets,
            scenario_game_options,
            |controller, layout, roster| match phase {
                TouchPhase::Started if double_click => {
                    controller.pointer_double_click(point, layout, roster)
                }
                TouchPhase::Started => controller.touch_start(point, layout, roster),
                TouchPhase::Moved => controller.touch_move(point, layout, roster),
                TouchPhase::Ended => controller.touch_end(point, layout, roster, Instant::now()),
                TouchPhase::Cancelled => controller.touch_cancel(),
            },
        )
    }

    pub(crate) fn render_classic(
        &mut self,
        surface: &mut Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
        include_tooltips: bool,
        active: bool,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        let resources = assets.game_lobby_resources()?;
        let option_resources = assets.game_option_resources()?;
        let fonts = assets
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful lobby fonts are unavailable")?;
        let (_, _, options) = self.synchronize_classic_controller(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            scenario_game_options,
        );
        let result = if include_tooltips {
            self.controller.render(
                surface,
                &resources,
                &options,
                &option_resources,
                active,
                Some(gamma),
            )
        } else {
            self.controller.render_without_tooltips(
                surface,
                &resources,
                &options,
                &option_resources,
                active,
                Some(gamma),
            )
        };
        self.retain_controller_state();
        result
    }

    pub(crate) fn render_classic_tooltips(
        &mut self,
        surface: &mut Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        let resources = assets.game_lobby_resources()?;
        let option_resources = assets.game_option_resources()?;
        let fonts = assets
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful lobby fonts are unavailable")?;
        let (_, _, options) = self.synchronize_classic_controller(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            scenario_game_options,
        );
        self.controller.render_tooltips(
            surface,
            &resources,
            &options,
            &option_resources,
            true,
            Some(gamma),
        )
    }

    pub(crate) fn wheel_right_sheet(
        &mut self,
        delta: i32,
        surface: &Surface,
        assets: &FrontendAssets,
        scenario_game_options: &GameOptionButtons,
    ) -> Result<(bool, bool)> {
        let Some(point) = self.pointer else {
            return Ok((false, false));
        };
        let fonts = assets
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful lobby fonts are unavailable")?;
        let (layout, roster, _) = self.synchronize_classic_controller(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            scenario_game_options,
        );
        let contains = |rect: clonk_frontend::classic_gui::IntRect| {
            point.x >= rect.x as f32
                && point.y >= rect.y as f32
                && point.x < (rect.x + rect.w) as f32
                && point.y < (rect.y + rect.h) as f32
        };
        let scroll_window_captured =
            contains(layout.chat_log_client) || contains(layout.roster_client);
        self.controller.note_pointer_wheel();
        let outside_scroll_window = contains(layout.chat_log) && !contains(layout.chat_log_client)
            || contains(layout.roster) && !contains(layout.roster_client);
        let changed =
            !outside_scroll_window && self.controller.wheel(point, delta, &layout, &roster);
        self.retain_controller_state();
        Ok((changed, scroll_window_captured))
    }

    pub(crate) fn browse_chat_history(&mut self, older: bool, history: &VecDeque<String>) -> bool {
        self.chat_history_index += if older { 1 } else { -1 };
        let horizontal_scroll = self.chat_edit.horizontal_scroll;
        let text = usize::try_from(self.chat_history_index)
            .ok()
            .and_then(|index| history.get(index))
            .filter(|text| !text.is_empty())
            .cloned();
        let inserted = match text {
            Some(text) => {
                self.chat_edit = LobbyChatEditView {
                    caret: text.len(),
                    selection: (!text.is_empty()).then_some((0, text.len())),
                    text,
                    horizontal_scroll,
                    cursor_visible: true,
                };
                true
            }
            None => {
                self.chat_history_index = -1;
                lobby_chat_clear_preserving_scroll(&mut self.chat_edit);
                false
            }
        };
        self.controller.set_chat_edit_view(self.chat_edit.clone());
        inserted
    }

    fn take_chat_submission(&mut self) -> String {
        let text = self.chat_edit.text.clone();
        lobby_chat_clear_preserving_scroll(&mut self.chat_edit);
        self.controller.set_chat_edit_view(self.chat_edit.clone());
        self.chat_history_index = -1;
        text
    }

    pub(crate) fn handle_key(&mut self, key: KeyCode, state: ElementState) -> Option<LobbyAction> {
        if state != ElementState::Pressed || self.controller.focus() != LobbyControl::ChatInput {
            return None;
        }
        match key {
            KeyCode::Escape => Some(LobbyAction::ExitRequested),
            KeyCode::Enter => Some(LobbyAction::SubmitMessage(self.take_chat_submission())),
            KeyCode::Up => {
                // The app routes history so lobby and running dialogs share
                // C4MessageInput's one process-local BackBuffer.
                Some(LobbyAction::ChatEdited)
            }
            KeyCode::Down => Some(LobbyAction::ChatEdited),
            KeyCode::Left => {
                let _ = lobby_chat_apply_edit_key(
                    &mut self.chat_edit,
                    LobbyChatEditKey::Left,
                    LobbyChatKeyModifiers::default(),
                );
                Some(LobbyAction::ChatEdited)
            }
            KeyCode::Right => {
                let _ = lobby_chat_apply_edit_key(
                    &mut self.chat_edit,
                    LobbyChatEditKey::Right,
                    LobbyChatKeyModifiers::default(),
                );
                Some(LobbyAction::ChatEdited)
            }
            KeyCode::Space => Some(LobbyAction::ChatEdited),
            _ => None,
        }
    }

    pub(crate) fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }
}

impl IngameMouseState {
    pub(crate) fn new(start: ViewportPointer, selection_frame: bool) -> Self {
        Self {
            start,
            last: start,
            selection_last: start,
            moved: false,
            selection_frame,
            selection_kind: IngameDragSelectionKind::Unknown,
            down_region: None,
            selection_cancelled_by_region: false,
            region_drag_started: false,
            world_drag_started: false,
            region_drag_cursor: None,
        }
    }

    pub(crate) fn update(&mut self, pointer: ViewportPointer) {
        self.last = pointer;
        if !self.moved {
            let dx = (self.last.world.x - self.start.world.x).abs();
            let dy = (self.last.world.y - self.start.world.y).abs();
            if dx >= MOUSE_DRAG_THRESHOLD || dy >= MOUSE_DRAG_THRESHOLD {
                self.moved = true;
            }
        }
    }

    fn update_with_fog(&mut self, pointer: ViewportPointer, fog_blocked: bool) -> bool {
        let was_moved = self.moved;
        self.update(pointer);
        if !was_moved && self.moved && fog_blocked {
            // DragNone refuses to enter a drag while the threshold-crossing
            // endpoint is fog-covered. A later visible endpoint may still
            // start the retained button gesture.
            self.moved = false;
            return false;
        }
        if self.moved && self.selection_frame && !fog_blocked {
            self.selection_last = pointer;
        }
        !was_moved && self.moved
    }
}

impl IngameButtonMouseState {
    pub(crate) fn new(
        start: ViewportPointer,
        down_target: Option<ObjectId>,
        down_region: bool,
    ) -> Self {
        Self {
            motion: IngameMouseState::new(start, down_target.is_none() && !down_region),
            down_target,
            down_region,
            down_cursor_nothing: false,
            down_cursor_help: false,
        }
    }

    pub(crate) fn update_with_fog(&mut self, pointer: ViewportPointer, fog_blocked: bool) -> bool {
        if self.down_cursor_help {
            self.motion.update(pointer);
            return false;
        }
        if self.down_cursor_nothing {
            self.motion.last = pointer;
            return false;
        }
        self.motion.update_with_fog(pointer, fog_blocked)
    }
}

impl MenuLayer {
    fn new(title: impl Into<String>, entries: Vec<FrontendScenario>) -> Self {
        Self {
            title: title.into(),
            entries,
            folder: None,
            style: MenuLayerStyle::Book,
        }
    }

    fn for_folder(folder: FrontendScenario) -> Self {
        Self {
            title: folder.title.clone(),
            entries: folder.children.clone(),
            folder: Some(folder),
            style: MenuLayerStyle::Book,
        }
    }
}

/// The port's enhanced scenario search is an accepted divergence from C++, so
/// its presentation has no oracle strings to mirror; these are port-owned
/// `IDS_` entries in `planet/System.c4g`, read from the same frozen active
/// language table every other startup string comes from
/// (clonk-org/clonk-rs#1175). The defaults are the English wording the search
/// shipped with, so a table that lacks the keys reads exactly as before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnhancedSearchResources {
    /// `IDS_MSG_SEARCHRESULTS` — matches, total, noun.
    pub(crate) results: String,
    /// `IDS_MSG_SEARCHNOMATCHES` — total, noun.
    pub(crate) no_matches: String,
    /// `IDS_MSG_SEARCHNORESULT` — the term that matched nothing.
    pub(crate) no_result: String,
    /// `IDS_MSG_SEARCHCLEARHINT`.
    pub(crate) clear_hint: String,
    /// `IDS_MSG_SEARCHSCENARIO` / `IDS_MSG_SEARCHSCENARIOS`. A separate noun,
    /// not the capitalized `IDS_TYPE_SCENARIO` this sentence would misuse, and
    /// two whole sentences: a language that inflects the count differently can
    /// reorder the format string without losing the singular/plural choice.
    pub(crate) scenario: String,
    pub(crate) scenarios: String,
}

impl Default for EnhancedSearchResources {
    fn default() -> Self {
        Self {
            results: "%d of %d %s".to_string(),
            no_matches: "No matches among %d %s".to_string(),
            no_result: "No scenarios match \"%s\".".to_string(),
            clear_hint: "Press Esc to clear search.".to_string(),
            scenario: "scenario".to_string(),
            scenarios: "scenarios".to_string(),
        }
    }
}

impl EnhancedSearchResources {
    fn noun(&self, count: usize) -> &str {
        if count == 1 {
            &self.scenario
        } else {
            &self.scenarios
        }
    }
}

impl MenuState {
    pub(crate) fn new(menu: StartupMenu, entries: Vec<FrontendScenario>) -> Self {
        let visible_entries = entries.clone();
        Self {
            menu,
            pointer_position: None,
            stack: vec![MenuLayer::new("Scenarios", entries)],
            visible_entries,
            visible_entry_contexts: Vec::new(),
            search_edit: SearchEditState::default(),
            applied_search_text: String::new(),
            enhanced_search_active: false,
            enhanced_search_total: 0,
            search_restore_selection: None,
            search_restore_scroll: None,
            rename_edit: None,
            scenario_list_scroll: 0,
            list_scroll_selection: None,
            selection_info_scroll: 0,
            scrollbar_interaction: None,
            definition_checkbox_enabled: false,
            definition_checkbox_checked: false,
            definition_checkbox_focused: false,
            dialog_focus: ScenselDialogFocus::List,
            include_back: true,
            scensel_title_present: true,
            scensel_title_topmost: false,
            enhanced_search_resources: EnhancedSearchResources::default(),
        }
    }

    /// Installs the active language table's enhanced-search wording. Called
    /// whenever the process language table is (re)loaded, so an Options
    /// language change reaches the next drawn caption.
    pub(crate) fn set_enhanced_search_resources(&mut self, resources: EnhancedSearchResources) {
        self.enhanced_search_resources = resources;
    }

    pub(crate) fn enhanced_search_clear_hint(&self) -> &str {
        &self.enhanced_search_resources.clear_hint
    }

    /// Switches Back-row injection and rebuilds the visible entries.
    pub(crate) fn set_include_back(&mut self, include_back: bool) {
        if self.include_back != include_back {
            self.include_back = include_back;
            self.refresh_menu_entries();
        }
    }

    /// The scenario behind the menu's selected row, if any.
    pub(crate) fn selected_scenario(&self) -> Option<&FrontendScenario> {
        if let Some(map) = self.current_map() {
            return map.selected_entry();
        }
        let offset = usize::from(self.include_back);
        let index = self.menu.selected_index()?.checked_sub(offset)?;
        self.visible_entries.get(index)
    }

    pub(crate) fn current_map(&self) -> Option<&MapFolderData> {
        match &self.stack.last()?.style {
            MenuLayerStyle::Book => None,
            MenuLayerStyle::Map(map) => Some(map),
        }
    }

    fn current_map_mut(&mut self) -> Option<&mut MapFolderData> {
        match &mut self.stack.last_mut()?.style {
            MenuLayerStyle::Book => None,
            MenuLayerStyle::Map(map) => Some(map),
        }
    }

    pub(crate) fn start_renaming_selected(&mut self, previous_focus: ScenselFocusSnapshot) -> bool {
        if self.rename_edit.is_some() || self.current_map().is_some() {
            return false;
        }
        let Some((identifier, mut title)) = self
            .selected_scenario()
            .map(|selected| (selected.identifier.clone(), selected.title.clone()))
        else {
            return false;
        };
        Markup::strip_markup(&mut title);
        self.search_edit.blur();
        self.definition_checkbox_focused = false;
        self.dialog_focus = ScenselDialogFocus::List;
        self.rename_edit = Some(ScenarioRenameState {
            identifier,
            edit: RenameEdit::new(title, Some(previous_focus)),
            last_click: None,
        });
        true
    }

    pub(crate) fn abort_renaming(&mut self) -> Option<ScenselFocusSnapshot> {
        let mut rename = self.rename_edit.take()?;
        rename.edit.abort();
        rename.edit.take_previous_focus()
    }

    pub(crate) fn resolve_renaming(
        &mut self,
        result: RenameEditResult,
    ) -> Option<ScenselFocusSnapshot> {
        let resolution = self.rename_edit.as_mut()?.edit.resolve(result);
        if resolution == RenameEditResolution::KeepEditing {
            return None;
        }
        let mut rename = self
            .rename_edit
            .take()
            .expect("finished rename state remains installed");
        rename.edit.take_previous_focus()
    }

    pub(crate) fn replace_discovered_entries(
        &mut self,
        entries: Vec<FrontendScenario>,
        selected_identifier: Option<&str>,
        select_first_when_missing: bool,
        apply_live_search: bool,
    ) -> Vec<StartupMenuAction> {
        let folder_identifiers = self
            .stack
            .iter()
            .filter_map(|layer| {
                layer
                    .folder
                    .as_ref()
                    .map(|folder| folder.identifier.clone())
            })
            .collect::<Vec<_>>();
        self.stack = vec![MenuLayer::new("Scenarios", entries)];
        for identifier in folder_identifiers {
            let Some(folder) = self
                .current_entries()
                .iter()
                .find(|entry| {
                    entry.identifier == identifier && matches!(entry.kind, ScenarioKind::Folder)
                })
                .cloned()
            else {
                break;
            };
            self.stack.push(MenuLayer::for_folder(folder));
        }
        // UpdateList reads the live edit text, not merely the last submitted
        // query, after F5/MissionAccess rebuilds the list.
        if apply_live_search {
            self.applied_search_text = self.search_edit.text().to_string();
        }
        self.rename_edit = None;
        self.pointer_position = None;
        self.scenario_list_scroll = 0;
        self.selection_info_scroll = 0;
        self.scrollbar_interaction = None;
        if self.enhanced_search_active {
            let _ = self.apply_enhanced_search();
        } else {
            self.refresh_menu_entries();
        }
        let selected = selected_identifier
            .and_then(|identifier| {
                self.visible_entries
                    .iter()
                    .position(|entry| entry.identifier == identifier)
            })
            .or_else(|| {
                (select_first_when_missing && !self.visible_entries.is_empty()).then_some(0)
            });
        let actions = selected
            .map(|index| index + usize::from(self.include_back))
            .and_then(|index| self.menu.select_entry_by_index(index).ok())
            .unwrap_or_default();
        self.sync_definition_checkbox_to_selection();
        actions
    }

    /// The folder whose contents are currently listed (None at root).
    pub(crate) fn current_folder(&self) -> Option<&FrontendScenario> {
        self.stack.last().and_then(|layer| layer.folder.as_ref())
    }

    /// The list caption: current folder name, or "Scenarios" at the root
    /// (C4StartupScenSelDlg::UpdateList, cpp:1527-1535).
    pub(crate) fn book_caption(&self) -> &str {
        self.current_folder()
            .map(|folder| folder.title.as_str())
            .unwrap_or("Scenarios")
    }

    pub(crate) fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    pub(crate) fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
    }

    pub(crate) fn current_entries(&self) -> &[FrontendScenario] {
        self.stack
            .last()
            .map(|layer| layer.entries.as_slice())
            .unwrap_or_default()
    }

    fn activation_path(&self, identifier: &str) -> Option<Vec<FrontendScenario>> {
        if self.enhanced_search_active {
            return find_frontend_entry_path(
                self.stack.first().map(|layer| layer.entries.as_slice())?,
                identifier,
            );
        }
        let mut path = self
            .stack
            .iter()
            .filter_map(|layer| layer.folder.clone())
            .collect::<Vec<_>>();
        path.extend(find_frontend_entry_path(
            self.current_entries(),
            identifier,
        )?);
        Some(path)
    }

    pub(crate) fn require_supported_activation(
        &self,
        identifier: &str,
    ) -> std::result::Result<Option<ScenarioKind>, ClassicParityBoundary> {
        Ok(self
            .activation_path(identifier)
            .and_then(|path| path.last().map(|entry| entry.kind)))
    }

    pub(crate) fn visible_entries(&self) -> &[FrontendScenario] {
        &self.visible_entries
    }

    pub(crate) fn search_result_context(&self, index: usize) -> Option<&str> {
        self.visible_entry_contexts
            .get(index)
            .and_then(|context| context.as_deref())
    }

    pub(crate) fn enhanced_search_caption(&self) -> Option<String> {
        self.enhanced_search_active.then(|| {
            let resources = &self.enhanced_search_resources;
            let total = self.enhanced_search_total.to_string();
            let noun = resources.noun(self.enhanced_search_total);
            if self.visible_entries.is_empty() {
                format_resource_string(resources.no_matches.clone(), &[&total, noun])
            } else {
                format_resource_string(
                    resources.results.clone(),
                    &[&self.visible_entries.len().to_string(), &total, noun],
                )
            }
        })
    }

    pub(crate) fn enhanced_search_empty_message(&self) -> Option<String> {
        (self.enhanced_search_active && self.visible_entries.is_empty()).then(|| {
            format_resource_string(
                self.enhanced_search_resources.no_result.clone(),
                &[self.applied_search_text.trim()],
            )
        })
    }

    pub(crate) fn set_search_text(&mut self, text: impl Into<String>) {
        self.search_edit.set_text(text);
    }

    pub(crate) fn insert_search_text(&mut self, text: &str) -> bool {
        self.search_edit.insert_text(text)
    }

    pub(crate) fn search_text(&self) -> &str {
        self.search_edit.text()
    }

    pub(crate) fn set_search_focused(&mut self, focused: bool) {
        if focused {
            self.definition_checkbox_focused = false;
            self.dialog_focus = ScenselDialogFocus::Search;
            self.search_edit.focus();
        } else {
            self.search_edit.blur();
            if self.dialog_focus == ScenselDialogFocus::Search {
                self.dialog_focus = ScenselDialogFocus::List;
            }
        }
    }

    pub(crate) fn search_focused(&self) -> bool {
        self.search_edit.is_focused()
    }

    pub(crate) fn sync_definition_checkbox_to_selection(&mut self) {
        let (enabled, checked) = self
            .selected_scenario()
            .filter(|entry| matches!(entry.kind, ScenarioKind::Scenario))
            .map(|entry| {
                (
                    !entry.local_only.unwrap_or(false),
                    entry.allow_user_change.unwrap_or(false),
                )
            })
            .unwrap_or((false, false));
        self.definition_checkbox_enabled = enabled;
        self.definition_checkbox_checked = checked;
        if !enabled {
            self.definition_checkbox_focused = false;
            if self.dialog_focus == ScenselDialogFocus::Definitions {
                self.dialog_focus = ScenselDialogFocus::List;
            }
        }
    }

    pub(crate) fn toggle_definition_checkbox(&mut self) -> bool {
        if !self.definition_checkbox_enabled {
            return false;
        }
        self.definition_checkbox_checked = !self.definition_checkbox_checked;
        true
    }

    pub(crate) fn set_definition_checkbox_focused(&mut self, focused: bool) -> bool {
        let focused = focused && self.definition_checkbox_enabled;
        if self.definition_checkbox_focused == focused {
            return false;
        }
        self.definition_checkbox_focused = focused;
        if focused {
            self.search_edit.blur();
            self.dialog_focus = ScenselDialogFocus::Definitions;
        } else if self.dialog_focus == ScenselDialogFocus::Definitions {
            self.dialog_focus = ScenselDialogFocus::List;
        }
        true
    }

    pub(crate) fn set_dialog_focus(&mut self, focus: ScenselDialogFocus) {
        let focus = if focus == ScenselDialogFocus::Definitions && !self.definition_checkbox_enabled
        {
            ScenselDialogFocus::List
        } else {
            focus
        };
        self.dialog_focus = focus;
        if focus == ScenselDialogFocus::Search {
            self.search_edit.focus();
        } else {
            self.search_edit.blur();
        }
        self.definition_checkbox_focused = focus == ScenselDialogFocus::Definitions;
    }

    pub(crate) const fn dialog_focus(&self) -> ScenselDialogFocus {
        self.dialog_focus
    }

    pub(crate) fn scenario_list_scroll(&self) -> i32 {
        self.scenario_list_scroll
    }

    pub(crate) fn scenario_list_max_scroll(&self, viewport_height: i32, pitch: i32) -> i32 {
        let row_count = self.visible_entries.len() + usize::from(self.include_back);
        let content_height = i32::try_from(row_count)
            .unwrap_or(i32::MAX)
            .saturating_mul(pitch)
            .saturating_sub(i32::from(row_count > 0));
        content_height.saturating_sub(viewport_height).max(0)
    }

    pub(crate) fn scroll_scenario_list_by(
        &mut self,
        amount: i32,
        viewport_height: i32,
        pitch: i32,
    ) -> bool {
        let max_scroll = self.scenario_list_max_scroll(viewport_height, pitch);
        let next = self
            .scenario_list_scroll
            .saturating_add(amount)
            .clamp(0, max_scroll);
        if next == self.scenario_list_scroll {
            return false;
        }
        self.scenario_list_scroll = next;
        self.list_scroll_selection = Some(self.menu.selected_index());
        true
    }

    pub(crate) fn ensure_list_selection_visible(
        &mut self,
        viewport_height: i32,
        pitch: i32,
        item_height: i32,
    ) {
        let Some(selection) = self.menu.selected_index() else {
            self.scenario_list_scroll = self
                .scenario_list_scroll
                .min(self.scenario_list_max_scroll(viewport_height, pitch));
            return;
        };
        let row_y = i32::try_from(selection)
            .unwrap_or(i32::MAX)
            .saturating_mul(pitch);
        if self.scenario_list_scroll > row_y {
            self.scenario_list_scroll = row_y;
        } else if self.scenario_list_scroll.saturating_add(viewport_height)
            < row_y.saturating_add(item_height)
        {
            self.scenario_list_scroll = row_y
                .saturating_add(item_height)
                .saturating_sub(viewport_height);
        }
        self.scenario_list_scroll = self
            .scenario_list_scroll
            .clamp(0, self.scenario_list_max_scroll(viewport_height, pitch));
        self.list_scroll_selection = Some(self.menu.selected_index());
    }

    pub(crate) fn select_list_index(&mut self, index: usize) -> Vec<StartupMenuAction> {
        match self.menu.select_entry_by_index(index) {
            Ok(actions) => actions,
            Err(err) => {
                tracing::error!(error = %err, index, "failed to select scenario list row");
                Vec::new()
            }
        }
    }

    pub(crate) fn move_list_selection_clamped(&mut self, delta: isize) -> Vec<StartupMenuAction> {
        let count = self.visible_entries.len() + usize::from(self.include_back);
        if count == 0 {
            return Vec::new();
        }
        let current = self.menu.selected_index().unwrap_or_else(|| {
            if delta.is_negative() {
                count - 1
            } else {
                0
            }
        });
        let next = current.saturating_add_signed(delta).min(count - 1);
        if next == current && self.menu.selected_index().is_some() {
            return Vec::new();
        }
        self.select_list_index(next)
    }

    pub(crate) fn select_list_home(&mut self) -> Vec<StartupMenuAction> {
        if self.visible_entries.is_empty() && !self.include_back {
            Vec::new()
        } else {
            self.select_list_index(0)
        }
    }

    pub(crate) fn select_list_end(&mut self) -> Vec<StartupMenuAction> {
        let count = self.visible_entries.len() + usize::from(self.include_back);
        count
            .checked_sub(1)
            .map(|index| self.select_list_index(index))
            .unwrap_or_default()
    }

    pub(crate) fn page_list_selection(
        &mut self,
        direction: i32,
        viewport_height: i32,
        pitch: i32,
        item_height: i32,
    ) -> Vec<StartupMenuAction> {
        let count = self.visible_entries.len() + usize::from(self.include_back);
        let Some(current) = self.menu.selected_index().filter(|index| *index < count) else {
            return if direction < 0 {
                self.select_list_end()
            } else {
                self.select_list_home()
            };
        };
        let pitch = pitch.max(1);
        let viewport_height = viewport_height.max(1);
        let max_scroll = self.scenario_list_max_scroll(viewport_height, pitch);
        let target = if direction >= 0 {
            let last_fully_visible = |scroll: i32| {
                scroll
                    .saturating_add(viewport_height)
                    .saturating_sub(item_height)
                    .max(0)
                    / pitch
            };
            let mut target = last_fully_visible(self.scenario_list_scroll)
                .max(current as i32)
                .min(count.saturating_sub(1) as i32) as usize;
            if target <= current && current + 1 < count {
                self.scenario_list_scroll = self
                    .scenario_list_scroll
                    .saturating_add(viewport_height)
                    .min(max_scroll);
                target = last_fully_visible(self.scenario_list_scroll)
                    .max(current.saturating_add(1) as i32)
                    .min(count.saturating_sub(1) as i32) as usize;
            }
            target
        } else {
            let first_fully_visible = |scroll: i32| scroll.saturating_add(pitch - 1).max(0) / pitch;
            let mut target = first_fully_visible(self.scenario_list_scroll)
                .max(0)
                .min(current as i32) as usize;
            if target >= current && current > 0 {
                self.scenario_list_scroll = self
                    .scenario_list_scroll
                    .saturating_sub(viewport_height)
                    .max(0);
                target = first_fully_visible(self.scenario_list_scroll)
                    .min(current.saturating_sub(1) as i32)
                    .max(0) as usize;
            }
            target
        };
        if target == current {
            return Vec::new();
        }
        let actions = self.select_list_index(target);
        self.list_scroll_selection = Some(self.menu.selected_index());
        actions
    }

    /// Mirrors `C4GUI::ListBox::CharIn`: search once from the row after the
    /// selection, wrap before the current row, and compare the raw first byte.
    pub(crate) fn select_list_character(&mut self, character: char) -> Vec<StartupMenuAction> {
        if self.dialog_focus != ScenselDialogFocus::List
            || self.current_map().is_some()
            || !character.is_ascii()
        {
            return Vec::new();
        }
        let offset = usize::from(self.include_back);
        let count = self.visible_entries.len() + offset;
        if count == 0 {
            return Vec::new();
        }

        let selected = self.menu.selected_index().filter(|index| *index < count);
        let start = selected.map_or(0, |index| (index + 1) % count);
        let candidates = if selected.is_some() {
            count.saturating_sub(1)
        } else {
            count
        };
        let input = character as u8;
        let target = (0..candidates).find_map(|delta| {
            let index = (start + delta) % count;
            let entry = self.visible_entries.get(index.checked_sub(offset)?)?;
            entry
                .title
                .as_bytes()
                .first()
                .is_some_and(|first| first.eq_ignore_ascii_case(&input))
                .then_some(index)
        });
        target
            .map(|index| self.select_list_index(index))
            .unwrap_or_default()
    }

    pub(crate) fn scroll_selection_info_by(
        &mut self,
        amount: i32,
        metrics: clonk_frontend::startup_scensel::SelectionInfoScrollMetrics,
    ) -> bool {
        let next = metrics.clamp_offset(self.selection_info_scroll.saturating_add(amount));
        if next == self.selection_info_scroll {
            return false;
        }
        self.selection_info_scroll = next;
        true
    }

    /// Applies the edit buffer like `OnSearchBarEnter -> UpdateList`
    /// (C4StartupScenSelDlg.h:434-437), preserving the selected entry by
    /// identity when it still survives and otherwise selecting the first.
    pub(crate) fn submit_search(&mut self) -> Vec<StartupMenuAction> {
        self.enhanced_search_active = false;
        self.enhanced_search_total = 0;
        self.search_restore_selection = None;
        self.search_restore_scroll = None;
        let old_selection = self
            .selected_scenario()
            .map(|entry| entry.identifier.clone());
        self.applied_search_text = self.search_edit.text().to_string();
        self.refresh_menu_entries();
        let index = old_selection
            .as_deref()
            .and_then(|identifier| {
                self.visible_entries
                    .iter()
                    .position(|entry| entry.identifier == identifier)
            })
            .or_else(|| (!self.visible_entries.is_empty()).then_some(0));
        index
            .map(|index| index + usize::from(self.include_back))
            .map(|index| match self.menu.select_entry_by_index(index) {
                Ok(actions) => actions,
                Err(err) => {
                    tracing::error!(error = %err, "failed to select submitted scenario search result");
                    Vec::new()
                }
            })
            .unwrap_or_else(|| {
                self.sync_definition_checkbox_to_selection();
                Vec::new()
            })
    }

    /// Applies the product search over every loaded non-folder descendant.
    /// `submit_search` remains the isolated C++ current-folder oracle path.
    pub(crate) fn apply_enhanced_search(&mut self) -> Vec<StartupMenuAction> {
        self.applied_search_text = self.search_edit.text().to_string();
        let normalized_query = normalize_scenario_search_text(&self.applied_search_text);
        if normalized_query.is_empty() {
            let restore_selection = self.search_restore_selection.take();
            let restore_scroll = self.search_restore_scroll.take();
            self.enhanced_search_active = false;
            self.enhanced_search_total = 0;
            self.applied_search_text.clear();
            self.refresh_menu_entries();
            self.scenario_list_scroll = restore_scroll.unwrap_or(0);
            let index = restore_selection
                .as_deref()
                .and_then(|identifier| {
                    self.visible_entries
                        .iter()
                        .position(|entry| entry.identifier == identifier)
                })
                .or_else(|| (!self.visible_entries.is_empty()).then_some(0));
            return index
                .map(|index| index + usize::from(self.include_back))
                .map(|index| match self.menu.select_entry_by_index(index) {
                    Ok(actions) => actions,
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to restore selection after clearing scenario search"
                        );
                        Vec::new()
                    }
                })
                .unwrap_or_default();
        }
        if !self.enhanced_search_active {
            self.search_restore_selection = self
                .selected_scenario()
                .map(|entry| entry.identifier.clone());
            self.search_restore_scroll = Some(self.scenario_list_scroll);
            self.enhanced_search_active = true;
        }
        let old_selection = self
            .selected_scenario()
            .map(|entry| entry.identifier.clone());
        let mut hits = Vec::new();
        let mut ancestors = Vec::new();
        let mut order = 0;
        if let Some(root) = self.stack.first() {
            collect_enhanced_scenario_search_matches(
                &root.entries,
                &normalized_query,
                &mut ancestors,
                &mut order,
                &mut hits,
            );
        }
        self.enhanced_search_total = order;
        hits.sort_by_key(|hit| (hit.rank, hit.order));
        self.visible_entry_contexts = hits
            .iter()
            .map(|hit| (!hit.context.is_empty()).then(|| hit.context.clone()))
            .collect();
        self.visible_entries = hits.into_iter().map(|hit| hit.entry).collect();
        let entries = build_menu_entries(&self.visible_entries, self.include_back);
        if let Err(err) = self.menu.set_entries(entries) {
            tracing::error!(error = %err, "failed to update startup menu search results");
        }
        self.list_scroll_selection = None;
        let index = old_selection
            .as_deref()
            .and_then(|identifier| {
                self.visible_entries
                    .iter()
                    .position(|entry| entry.identifier == identifier)
            })
            .or_else(|| (!self.visible_entries.is_empty()).then_some(0));
        index
            .map(|index| index + usize::from(self.include_back))
            .map(|index| match self.menu.select_entry_by_index(index) {
                Ok(actions) => actions,
                Err(err) => {
                    tracing::error!(error = %err, "failed to select enhanced scenario search result");
                    Vec::new()
                }
            })
            .unwrap_or_else(|| {
                self.sync_definition_checkbox_to_selection();
                Vec::new()
            })
    }

    pub(crate) fn clear_search(&mut self) {
        self.search_edit = SearchEditState::default();
        self.applied_search_text.clear();
        self.enhanced_search_active = false;
        self.enhanced_search_total = 0;
        self.search_restore_selection = None;
        self.search_restore_scroll = None;
        self.visible_entry_contexts.clear();
    }

    pub(crate) fn menu(&mut self) -> &mut StartupMenu {
        &mut self.menu
    }

    pub(crate) fn configure_current_folder_map(
        &mut self,
        show_folder_maps: bool,
        screen_width: i32,
        screen_height: i32,
        mission_access: &MissionAccessStore,
        languages: &[String],
    ) -> bool {
        let Some(layer) = self.stack.last_mut() else {
            return false;
        };
        layer.style = MenuLayerStyle::Book;
        if !show_folder_maps {
            if !self.scensel_title_present {
                self.scensel_title_present = true;
                self.scensel_title_topmost = true;
            }
            return false;
        }
        let Some(folder) = layer.folder.clone() else {
            if !self.scensel_title_present {
                self.scensel_title_present = true;
                self.scensel_title_topmost = true;
            }
            return false;
        };
        let Some(map) = load_map_folder_data(
            &folder,
            screen_width,
            screen_height,
            mission_access,
            languages,
        ) else {
            if !self.scensel_title_present {
                self.scensel_title_present = true;
                self.scensel_title_topmost = true;
            }
            return false;
        };
        if map.hide_title {
            self.scensel_title_present = false;
        }
        layer.style = MenuLayerStyle::Map(map);
        self.selection_info_scroll = 0;
        self.scrollbar_interaction = None;
        self.sync_definition_checkbox_to_selection();
        true
    }

    pub(crate) fn activate_map_button(&mut self, index: usize) -> Option<StartupMenuAction> {
        let map = self.current_map_mut()?;
        let button = map.scenarios.get(index)?;
        let entry = button.entry.clone();
        let previous_identifier = map.selected_entry().map(|entry| entry.identifier.clone());
        let single_click = button.single_click;
        map.selected_button = Some(index);
        let entry = entry?;
        let summary = clonk_frontend::ScenarioSummary {
            identifier: entry.identifier.clone(),
            title: entry.title.clone(),
            kind: entry.kind,
        };
        if single_click || previous_identifier.as_deref() == Some(entry.identifier.as_str()) {
            Some(if summary.kind == ScenarioKind::Folder {
                StartupMenuAction::OpenEntry(summary)
            } else {
                StartupMenuAction::StartScenario(summary)
            })
        } else {
            Some(StartupMenuAction::SelectionChanged(summary))
        }
    }

    pub(crate) fn start_selected_map_scenario(&self) -> Option<StartupMenuAction> {
        let entry = self.current_map()?.selected_entry()?;
        let summary = clonk_frontend::ScenarioSummary {
            identifier: entry.identifier.clone(),
            title: entry.title.clone(),
            kind: entry.kind,
        };
        Some(if summary.kind == ScenarioKind::Folder {
            StartupMenuAction::OpenEntry(summary)
        } else {
            StartupMenuAction::StartScenario(summary)
        })
    }

    pub(crate) fn deselect_map(&mut self) -> bool {
        let Some(map) = self.current_map_mut() else {
            return false;
        };
        let changed = map.selected_button.take().is_some();
        self.selection_info_scroll = 0;
        self.sync_definition_checkbox_to_selection();
        changed
    }

    pub(crate) fn enter_folder(&mut self, identifier: &str) {
        let Some(folder) = self
            .current_entries()
            .iter()
            .find(|entry| {
                entry.identifier == identifier && matches!(entry.kind, ScenarioKind::Folder)
            })
            .cloned()
        else {
            return;
        };
        self.stack.push(MenuLayer::for_folder(folder));
        self.pointer_position = None;
        self.scenario_list_scroll = 0;
        self.selection_info_scroll = 0;
        self.scrollbar_interaction = None;
        self.definition_checkbox_focused = false;
        self.dialog_focus = ScenselDialogFocus::List;
        self.clear_search();
        self.refresh_menu_entries();
    }

    pub(crate) fn leave_folder(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        self.stack.pop();
        self.pointer_position = None;
        self.scenario_list_scroll = 0;
        self.selection_info_scroll = 0;
        self.scrollbar_interaction = None;
        self.definition_checkbox_focused = false;
        self.dialog_focus = ScenselDialogFocus::List;
        self.clear_search();
        self.refresh_menu_entries();
    }

    pub(crate) fn refresh_menu_entries(&mut self) {
        let needle = self.applied_search_text.to_lowercase();
        self.visible_entries = if needle.is_empty() {
            self.current_entries().to_vec()
        } else {
            let mut matches = Vec::new();
            collect_current_folder_search_matches(self.current_entries(), &needle, &mut matches);
            matches
        };
        self.visible_entry_contexts = vec![None; self.visible_entries.len()];
        let entries = build_menu_entries(&self.visible_entries, self.include_back);
        if let Err(err) = self.menu.set_entries(entries) {
            tracing::error!(error = %err, "failed to update startup menu entries");
        }
        self.list_scroll_selection = None;
    }

    pub(crate) fn label_path(&self) -> String {
        if self.stack.is_empty() {
            return String::new();
        }
        self.stack
            .iter()
            .map(|layer| layer.title.clone())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub(crate) fn select_default_entry(&mut self) -> Vec<StartupMenuAction> {
        if self.current_map().is_some() {
            self.sync_definition_checkbox_to_selection();
            return Vec::new();
        }
        if self.current_entries().is_empty() {
            return Vec::new();
        }
        // The first real entry (past the Back row when present), mirroring
        // SelectFirstEntry (C4StartupScenSelDlg.cpp:1536-1537).
        let target_index = usize::from(self.include_back);
        match self.menu.select_entry_by_index(target_index) {
            Ok(actions) => actions,
            Err(err) => {
                tracing::error!(error = %err, "failed to select default scenario entry");
                Vec::new()
            }
        }
    }
}

struct EnhancedScenarioSearchHit {
    entry: FrontendScenario,
    context: String,
    rank: u16,
    order: usize,
}

fn normalize_scenario_search_text(text: &str) -> String {
    let mut normalized = String::new();
    let mut separated = true;
    for character in text
        .nfd()
        .filter(|character| !is_combining_mark(*character))
    {
        for lowercase in character.to_lowercase() {
            if lowercase.is_alphanumeric() {
                normalized.push(lowercase);
                separated = false;
            } else if !separated {
                normalized.push(' ');
                separated = true;
            }
        }
    }
    if separated {
        normalized.pop();
    }
    normalized
}

fn normalize_scenario_search_field(text: &str) -> String {
    let mut text = text.to_string();
    Markup::strip_markup(&mut text);
    normalize_scenario_search_text(&text)
}

fn scenario_search_field_contains_all(field: &str, terms: &[&str]) -> bool {
    terms.iter().all(|term| field.contains(term))
}

fn scenario_search_fuzzy_title_score(title: &str, terms: &[&str]) -> Option<u16> {
    let title_terms = title.split_whitespace().collect::<Vec<_>>();
    terms.iter().try_fold(0_u16, |score, term| {
        if title_terms.iter().any(|candidate| candidate.contains(term)) {
            return Some(score);
        }
        let length = term.chars().count();
        let threshold = if term.chars().any(|character| character.is_numeric()) || length < 4 {
            return None;
        } else if length >= 8 {
            2
        } else {
            1
        };
        title_terms
            .iter()
            .map(|candidate| damerau_levenshtein(term, candidate))
            .min()
            .filter(|distance| *distance <= threshold)
            .and_then(|distance| u16::try_from(distance).ok())
            .map(|distance| score.saturating_add(distance))
    })
}

fn collect_enhanced_scenario_search_matches(
    entries: &[FrontendScenario],
    query: &str,
    ancestors: &mut Vec<String>,
    order: &mut usize,
    matches: &mut Vec<EnhancedScenarioSearchHit>,
) {
    let terms = query.split_whitespace().collect::<Vec<_>>();
    for entry in entries {
        if matches!(entry.kind, ScenarioKind::Folder) {
            let mut title = entry.title.clone();
            Markup::strip_markup(&mut title);
            ancestors.push(title);
            collect_enhanced_scenario_search_matches(
                &entry.children,
                query,
                ancestors,
                order,
                matches,
            );
            ancestors.pop();
            continue;
        }
        let entry_order = *order;
        *order = order.saturating_add(1);
        let title = normalize_scenario_search_field(&entry.title);
        let identifier = normalize_scenario_search_text(&entry.identifier);
        let context = normalize_scenario_search_text(&ancestors.join(" "));
        let author = entry
            .author
            .as_deref()
            .map(normalize_scenario_search_field)
            .unwrap_or_default();
        let description = entry
            .description
            .as_deref()
            .map(normalize_scenario_search_field)
            .unwrap_or_default();
        let all_fields = [
            title.as_str(),
            identifier.as_str(),
            context.as_str(),
            author.as_str(),
            description.as_str(),
        ]
        .join(" ");
        let rank = if title == query {
            Some(0)
        } else if title.starts_with(query) {
            Some(1)
        } else if title.contains(query) {
            Some(2)
        } else if scenario_search_field_contains_all(&title, &terms) {
            Some(3)
        } else if scenario_search_field_contains_all(&identifier, &terms) {
            Some(4)
        } else if scenario_search_field_contains_all(&context, &terms) {
            Some(5)
        } else if scenario_search_field_contains_all(&author, &terms) {
            Some(6)
        } else if scenario_search_field_contains_all(&description, &terms) {
            Some(7)
        } else if scenario_search_field_contains_all(&all_fields, &terms) {
            Some(8)
        } else {
            scenario_search_fuzzy_title_score(&title, &terms)
                .map(|distance| 100_u16.saturating_add(distance))
        };
        if let Some(rank) = rank {
            matches.push(EnhancedScenarioSearchHit {
                entry: entry.clone(),
                context: ancestors.join(" / "),
                rank,
                order: entry_order,
            });
        }
    }
}

fn collect_current_folder_search_matches(
    entries: &[FrontendScenario],
    needle: &str,
    matches: &mut Vec<FrontendScenario>,
) {
    for entry in entries {
        let mut name = entry.title.clone();
        Markup::strip_markup(&mut name);
        if name.to_lowercase().contains(needle) {
            matches.push(entry.clone());
        }
    }
}

fn find_frontend_entry_path(
    entries: &[FrontendScenario],
    identifier: &str,
) -> Option<Vec<FrontendScenario>> {
    for entry in entries {
        if entry.identifier == identifier {
            return Some(vec![entry.clone()]);
        }
        if let Some(mut descendants) = find_frontend_entry_path(&entry.children, identifier) {
            let mut path = Vec::with_capacity(descendants.len() + 1);
            path.push(entry.clone());
            path.append(&mut descendants);
            return Some(path);
        }
    }
    None
}

#[derive(Debug, Default)]
pub(crate) struct ParsedMapFolder {
    scenario_info_area: MapFolderRect,
    pub(crate) min_res_x: i32,
    pub(crate) min_res_y: i32,
    fullscreen_background: bool,
    hide_title: bool,
    pub(crate) scenarios: Vec<ParsedMapFolderScenario>,
    access_overlays: Vec<ParsedMapFolderAccess>,
}

#[derive(Debug)]
pub(crate) struct ParsedMapFolderScenario {
    pub(crate) filename: String,
    base_image: String,
    overlay_image: String,
    single_click: bool,
    area: MapFolderRect,
    title: String,
    title_font_size: i32,
    title_color_inactive: u32,
    title_color_active: u32,
    title_offset_x: i32,
    title_offset_y: i32,
    title_align: u8,
    title_use_book_font: bool,
    image_dump: bool,
}

impl Default for ParsedMapFolderScenario {
    fn default() -> Self {
        Self {
            filename: String::new(),
            base_image: String::new(),
            overlay_image: String::new(),
            single_click: false,
            area: MapFolderRect::default(),
            title: String::new(),
            title_font_size: 20,
            title_color_inactive: 0x7fff_ffff,
            title_color_active: 0x0fff_ffff,
            title_offset_x: 0,
            title_offset_y: 0,
            title_align: 1,
            title_use_book_font: true,
            image_dump: false,
        }
    }
}

#[derive(Debug, Default)]
struct ParsedMapFolderAccess {
    password: String,
    overlay_image: String,
    area: MapFolderRect,
}

#[derive(Clone, Copy)]
enum ParsedMapFolderSection {
    None,
    Root,
    Scenario(usize),
    Access(usize),
}

pub(crate) fn parse_map_folder(text: &str) -> Result<ParsedMapFolder> {
    let mut parsed = ParsedMapFolder::default();
    let mut section = ParsedMapFolderSection::None;
    let mut section_indentation = 0;
    let mut root_active = false;
    let mut saw_root = false;
    let mut seen_values = HashSet::new();

    for raw_line in text.lines() {
        let raw_line = raw_line.trim_end_matches('\r');
        let indentation = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = raw_line[indentation..].trim_end();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            if indentation == 0 {
                root_active = name == "FolderMap";
                section_indentation = 0;
                section = if root_active {
                    saw_root = true;
                    ParsedMapFolderSection::Root
                } else {
                    ParsedMapFolderSection::None
                };
                continue;
            }
            if !root_active || (indentation > section_indentation && section_indentation > 0) {
                section = ParsedMapFolderSection::None;
                section_indentation = indentation;
                continue;
            }
            section_indentation = indentation;
            section = match name {
                "Scenario" => {
                    parsed.scenarios.push(ParsedMapFolderScenario::default());
                    ParsedMapFolderSection::Scenario(parsed.scenarios.len() - 1)
                }
                "AccessGfx" => {
                    parsed
                        .access_overlays
                        .push(ParsedMapFolderAccess::default());
                    ParsedMapFolderSection::Access(parsed.access_overlays.len() - 1)
                }
                _ => ParsedMapFolderSection::None,
            };
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = map_folder_string(raw_value);
        if indentation == 0 && root_active {
            section = ParsedMapFolderSection::Root;
            section_indentation = 0;
        }
        let section_key = match section {
            ParsedMapFolderSection::None => None,
            ParsedMapFolderSection::Root if indentation == 0 => Some((0_u8, 0_usize)),
            ParsedMapFolderSection::Root => None,
            ParsedMapFolderSection::Scenario(index)
                if indentation >= section_indentation && section_indentation > 0 =>
            {
                Some((1, index))
            }
            ParsedMapFolderSection::Access(index)
                if indentation >= section_indentation && section_indentation > 0 =>
            {
                Some((2, index))
            }
            ParsedMapFolderSection::Scenario(_) | ParsedMapFolderSection::Access(_) => None,
        };
        let Some((section_kind, section_index)) = section_key else {
            continue;
        };
        if !seen_values.insert((section_kind, section_index, key.to_string())) {
            continue;
        }
        match section {
            ParsedMapFolderSection::None => {}
            ParsedMapFolderSection::Root => match key {
                "ScenInfoArea" => parsed.scenario_info_area = parse_map_folder_rect(&value)?,
                "MinResX" => parsed.min_res_x = parse_map_folder_i32(&value, key)?,
                "MinResY" => parsed.min_res_y = parse_map_folder_i32(&value, key)?,
                "FullscreenBG" => {
                    parsed.fullscreen_background = parse_map_folder_bool(&value, key)?
                }
                "HideTitle" => parsed.hide_title = parse_map_folder_bool(&value, key)?,
                _ => {}
            },
            ParsedMapFolderSection::Scenario(index) => {
                let scenario = &mut parsed.scenarios[index];
                match key {
                    "File" => scenario.filename = value,
                    "BaseImage" => scenario.base_image = value,
                    "OverlayImage" => scenario.overlay_image = value,
                    "SingleClick" => scenario.single_click = parse_map_folder_bool(&value, key)?,
                    "Area" => scenario.area = parse_map_folder_rect(&value)?,
                    "Title" => scenario.title = value,
                    "TitleFontSize" => {
                        scenario.title_font_size = parse_map_folder_i32(&value, key)?
                    }
                    "TitleColorInactive" => {
                        scenario.title_color_inactive = parse_map_folder_u32(&value, key)?
                    }
                    "TitleColorActive" => {
                        scenario.title_color_active = parse_map_folder_u32(&value, key)?
                    }
                    "TitleOffX" => scenario.title_offset_x = parse_map_folder_i32(&value, key)?,
                    "TitleOffY" => scenario.title_offset_y = parse_map_folder_i32(&value, key)?,
                    "TitleAlign" => {
                        scenario.title_align = value
                            .parse::<u8>()
                            .with_context(|| format!("invalid FolderMap {key} value `{value}`"))?
                    }
                    "TitleUseBookFont" => {
                        scenario.title_use_book_font = parse_map_folder_bool(&value, key)?
                    }
                    "ImageDump" => scenario.image_dump = parse_map_folder_bool(&value, key)?,
                    _ => {}
                }
            }
            ParsedMapFolderSection::Access(index) => {
                let access = &mut parsed.access_overlays[index];
                match key {
                    "Access" => access.password = value,
                    "OverlayImage" => access.overlay_image = value,
                    "Area" => access.area = parse_map_folder_rect(&value)?,
                    _ => {}
                }
            }
        }
    }

    anyhow::ensure!(saw_root, "FolderMap.txt has no [FolderMap] root");
    Ok(parsed)
}

fn map_folder_string(raw: &str) -> String {
    let value = raw.trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

fn parse_map_folder_i32(value: &str, key: &str) -> Result<i32> {
    value
        .parse::<i32>()
        .with_context(|| format!("invalid FolderMap {key} value `{value}`"))
}

fn parse_map_folder_u32(value: &str, key: &str) -> Result<u32> {
    let parsed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| u32::from_str_radix(hex, 16))
        .unwrap_or_else(|| value.parse::<u32>());
    parsed.with_context(|| format!("invalid FolderMap {key} value `{value}`"))
}

fn parse_map_folder_bool(value: &str, key: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => anyhow::bail!("invalid FolderMap {key} value `{value}`"),
    }
}

fn parse_map_folder_rect(value: &str) -> Result<MapFolderRect> {
    let values = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("invalid FolderMap rectangle `{value}`"))?;
    anyhow::ensure!(
        values.len() == 4,
        "invalid FolderMap rectangle `{value}`: expected four values"
    );
    Ok(MapFolderRect {
        x: values[0],
        y: values[1],
        w: values[2],
        h: values[3],
    })
}

pub(crate) fn load_map_folder_data(
    folder: &FrontendScenario,
    screen_width: i32,
    screen_height: i32,
    mission_access: &MissionAccessStore,
    languages: &[String],
) -> Option<MapFolderData> {
    if !folder
        .path
        .as_deref()
        .and_then(Path::extension)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
    {
        return None;
    }

    let mut seen = HashSet::new();
    for path in folder.path.iter().chain(folder.source_paths.iter()) {
        if !seen.insert(scenario_root_key(path)) {
            continue;
        }
        let group = match open_group_path_for_folder_map(path) {
            Ok(group) => group,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "failed to inspect FolderMap group");
                continue;
            }
        };
        let bytes = match group.load_entry_string("FolderMap.txt") {
            Ok(bytes) => bytes,
            Err(GroupError::EntryNotFound(_)) => continue,
            Err(GroupError::Io(ref error)) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "FolderMap data could not be loaded; using book view");
                continue;
            }
        };
        let text = clonk_resources::decode_legacy_script_text(&bytes);
        match build_map_folder_data(
            path,
            &group,
            folder,
            &text,
            screen_width,
            screen_height,
            mission_access,
            languages,
        ) {
            Ok(map) => return Some(map),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "FolderMap could not be initialized; using book view");
            }
        }
    }
    None
}

fn build_map_folder_data(
    source_path: &Path,
    group: &Group,
    folder: &FrontendScenario,
    text: &str,
    screen_width: i32,
    screen_height: i32,
    mission_access: &MissionAccessStore,
    languages: &[String],
) -> Result<MapFolderData> {
    let text = clonk_resources::localize_script_source(group, text, languages)?;
    let parsed = parse_map_folder(&text)?;
    anyhow::ensure!(
        parsed.min_res_x == 0 || parsed.min_res_x <= screen_width,
        "FolderMap requires width {}, current width is {screen_width}",
        parsed.min_res_x
    );
    anyhow::ensure!(
        parsed.min_res_y == 0 || parsed.min_res_y <= screen_height,
        "FolderMap requires height {}, current height is {screen_height}",
        parsed.min_res_y
    );

    let background = load_map_folder_background(group)?;
    let mut scenario_info_area = parsed.scenario_info_area;
    if scenario_info_area.w == 0 {
        let width = i32::try_from(background.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(background.height()).unwrap_or(i32::MAX);
        scenario_info_area = MapFolderRect {
            x: width * 2 / 3,
            y: height / 16,
            w: width / 3,
            h: height * 7 / 8,
        };
    }

    let mut scenarios = Vec::with_capacity(parsed.scenarios.len());
    for scenario in parsed.scenarios {
        let entry = folder.children.iter().find(|entry| {
            entry
                .identifier
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(&scenario.filename))
        });
        let replacement = entry
            .map(|entry| entry.title.as_str())
            .unwrap_or("<c ff0000>ERROR</c>");
        let title = scenario.title.replace("TITLE", replacement);
        let base_image = if scenario.image_dump {
            // `Load` blits the scenario area out of the background into a
            // fresh facet, saves it under BaseImage, then `continue`s past the
            // ordinary base load - a failed dump only logs
            // (C4StartupScenSelDlg.cpp:145-161).
            if let Err(error) = dump_map_folder_base_image(
                source_path,
                &background,
                scenario.area,
                &scenario.base_image,
            ) {
                tracing::warn!(
                    image = %scenario.base_image,
                    %error,
                    "FolderMap ImageDump could not be written"
                );
            }
            None
        } else if scenario.base_image.is_empty() {
            None
        } else {
            Some(load_map_folder_image(group, &scenario.base_image)?)
        };
        let overlay_image = if scenario.overlay_image.is_empty() {
            None
        } else {
            Some(load_map_folder_image(group, &scenario.overlay_image)?)
        };
        if entry.is_some_and(|entry| !entry.has_mission_access(mission_access)) {
            continue;
        }
        scenarios.push(MapFolderScenarioButton {
            entry: entry.cloned(),
            base_image,
            overlay_image,
            single_click: scenario.single_click,
            area: scenario.area,
            title,
            title_font_size: scenario.title_font_size,
            title_color_inactive: scenario.title_color_inactive,
            title_color_active: scenario.title_color_active,
            title_offset_x: scenario.title_offset_x,
            title_offset_y: scenario.title_offset_y,
            title_align: scenario.title_align,
            title_use_book_font: scenario.title_use_book_font,
        });
    }

    let mut access_overlays = Vec::new();
    for access in parsed.access_overlays {
        let image = (!access.overlay_image.is_empty())
            .then(|| load_map_folder_image(group, &access.overlay_image))
            .transpose()?;
        if access.password.is_empty() || mission_access.contains(&access.password) {
            access_overlays.push(MapFolderAccessOverlay {
                image,
                area: access.area,
            });
        }
    }

    Ok(MapFolderData {
        source_path: source_path.to_path_buf(),
        background,
        scenario_info_area,
        fullscreen_background: parsed.fullscreen_background,
        hide_title: parsed.hide_title,
        scenarios,
        access_overlays,
        selected_button: None,
    })
}

/// `C4MapFolderData::Load`'s developer ImageDump: crop `Area` out of the
/// FolderMap background and write it beside the map as a PNG with alpha
/// (C4StartupScenSelDlg.cpp:147-158). C++ blits into a facet of exactly the
/// requested size, so a window reaching past the background edge keeps the
/// uncovered pixels transparent.
fn dump_map_folder_base_image(
    source_path: &Path,
    background: &ImageData,
    area: MapFolderRect,
    base_image: &str,
) -> Result<()> {
    anyhow::ensure!(!base_image.is_empty(), "ImageDump has no BaseImage name");
    let width = u32::try_from(area.w).context("ImageDump width is negative")?;
    let height = u32::try_from(area.h).context("ImageDump height is negative")?;
    anyhow::ensure!(width > 0 && height > 0, "ImageDump area is empty");
    let mut dump = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let source_x = area.x + x as i32;
            let source_y = area.y + y as i32;
            if source_x < 0 || source_y < 0 {
                continue;
            }
            let (Ok(source_x), Ok(source_y)) = (u32::try_from(source_x), u32::try_from(source_y))
            else {
                continue;
            };
            if source_x >= background.width() || source_y >= background.height() {
                continue;
            }
            let offset = ((source_y * background.width() + source_x) * 4) as usize;
            let Some(pixel) = background.pixels().get(offset..offset + 4) else {
                continue;
            };
            dump.put_pixel(x, y, image::Rgba([pixel[0], pixel[1], pixel[2], pixel[3]]));
        }
    }
    let destination = source_path.join(base_image);
    dump.save(&destination)
        .with_context(|| format!("writing ImageDump to {}", destination.display()))
}

fn load_map_folder_background(group: &Group) -> Result<ImageData> {
    for extension in ["png", "bmp", "jpeg", "jpg"] {
        let name = format!("FolderMap.{extension}");
        match group.read_file(&name) {
            Ok(bytes) => return decode_map_folder_image(&name, &bytes),
            Err(GroupError::EntryNotFound(_)) => {}
            Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("loading {name}")),
        }
    }
    anyhow::bail!("FolderMap background graphic is missing")
}

fn load_map_folder_image(group: &Group, name: &str) -> Result<ImageData> {
    if Path::new(name).extension().is_none() {
        for extension in ["png", "bmp", "jpeg", "jpg"] {
            let candidate = format!("{name}.{extension}");
            match group.read_file(&candidate) {
                Ok(bytes) => return decode_map_folder_image(&candidate, &bytes),
                Err(GroupError::EntryNotFound(_)) => {}
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("loading FolderMap image `{candidate}`"));
                }
            }
        }
        anyhow::bail!("FolderMap image `{name}` is missing")
    }
    let bytes = group
        .read_file(name)
        .with_context(|| format!("loading FolderMap image `{name}`"))?;
    decode_map_folder_image(name, &bytes)
}

fn decode_map_folder_image(name: &str, bytes: &[u8]) -> Result<ImageData> {
    let image = clonk_resources::load_image_from_memory(bytes)
        .with_context(|| format!("decoding FolderMap image `{name}`"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel[..3].fill(0);
        }
    }
    Ok(ImageData::new(width, height, pixels))
}

pub(crate) fn open_group_path_for_folder_map(
    path: &Path,
) -> std::result::Result<Group, GroupError> {
    if path.exists() {
        return Group::open(path);
    }

    // Packed child groups expose a logical `outer.c4f/inner.c4f` root even
    // though only `outer.c4f` exists in the filesystem. Reopen the nearest
    // real ancestor and traverse children through the group's
    // case-insensitive index instead of treating the logical path as a file.
    let mut ancestor = path.to_path_buf();
    let mut children = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name().map(|name| name.to_os_string()) else {
            return Err(GroupError::Missing(path.to_path_buf()));
        };
        children.push(PathBuf::from(name));
        if !ancestor.pop() {
            return Err(GroupError::Missing(path.to_path_buf()));
        }
    }
    let mut group = Group::open(&ancestor)?;
    for child in children.iter().rev() {
        group = open_child_flexible(&group, child)?
            .ok_or_else(|| GroupError::EntryNotFound(path.to_path_buf()))?;
    }
    Ok(group)
}

pub(crate) fn packed_group_bytes(
    path: &Path,
    maker: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    // A packed top-level group is copied byte-for-byte. C4Group::raw_image is
    // the uncompressed nested image and therefore is not a standalone file.
    if path.is_file() {
        return fs::read(path).map_err(|error| error.to_string());
    }
    if path.is_dir() {
        let group = Group::open(path).map_err(|error| error.to_string())?;
        let mut group = MutableGroup::from_group(&group).map_err(|error| error.to_string())?;
        if !maker.is_empty() {
            group.set_maker_bytes_recursively(maker);
        }
        return group.pack().map_err(|error| error.to_string());
    }

    // A logical child below a packed ancestor has no physical path. Re-wrap
    // its exact raw image in C4Group's standalone gzip envelope.
    let group = open_group_path_for_folder_map(path).map_err(|error| error.to_string())?;
    let image = group.raw_image().map_err(|error| error.to_string())?;
    clonk_resources::compress_c4group_image(&image).map_err(|error| error.to_string())
}

impl MainMenuState {
    pub(crate) fn new(menu: StartupMainMenu, participants_label: String) -> Self {
        Self {
            menu,
            participants_label,
        }
    }

    pub(crate) fn pointer_position(&self) -> Option<GuiPoint> {
        self.menu.pointer_position()
    }

    pub(crate) fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.menu.set_pointer_position(position);
    }

    pub(crate) fn participants_contains(&self, point: GuiPoint) -> bool {
        self.menu
            .participants_contains(&self.participants_label, point)
    }

    pub(crate) fn tooltip_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        self.menu.tooltip_at(&self.participants_label, point)
    }

    pub(crate) fn handle_pointer_move(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_move(point)
    }

    pub(crate) fn handle_pointer_down(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_down(point)
    }

    pub(crate) fn handle_pointer_up(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_up(point)
    }

    pub(crate) fn handle_key_down(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        self.menu.handle_key_down(key)
    }

    pub(crate) fn handle_key_up(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        self.menu.handle_key_up(key)
    }

    pub(crate) fn pointer_left(&mut self) {
        self.menu.pointer_left();
    }

    pub(crate) fn resize(&mut self, width: f32, height: f32) {
        self.menu.resize(width, height);
    }

    pub(crate) fn render(&mut self, surface: &mut Surface, draw_focus: bool) {
        self.menu
            .render_with_draw_focus(surface, &self.participants_label, draw_focus);
    }

    pub(crate) fn render_chrome(&mut self, surface: &mut Surface) {
        self.menu.render_chrome(surface);
    }

    pub(crate) fn render_native_text(
        &self,
        surface: &mut Surface,
        fonts: &clonk_frontend::clonk_fonts::NativeClonkFontSet,
        physical_offset: (i32, i32),
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        self.menu.render_native_text_with_offset(
            surface,
            fonts,
            &self.participants_label,
            physical_offset,
            gamma,
        );
    }

    pub(crate) fn update_participants_label(&mut self, label: String) {
        self.participants_label = label;
    }
}

pub(crate) const PLACEHOLDER_PREVIEW_WIDTH: u32 = 320;
pub(crate) const PLACEHOLDER_PREVIEW_HEIGHT: u32 = 200;

pub(crate) fn generate_preview_placeholder(kind: ScenarioKind, title: &str) -> ImageData {
    let (top, bottom, accent) = preview_palette(kind);
    let mut pixels =
        vec![0u8; (PLACEHOLDER_PREVIEW_WIDTH * PLACEHOLDER_PREVIEW_HEIGHT * 4) as usize];

    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    let seed = hasher.finish();

    let stripe_spacing = 5 + (seed % 5) as u32;
    let stripe_offset = if stripe_spacing == 0 {
        0
    } else {
        (seed as u32) % stripe_spacing
    };
    let noise_seed = seed.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
    let highlight_start = PLACEHOLDER_PREVIEW_HEIGHT.saturating_sub(48);

    for y in 0..PLACEHOLDER_PREVIEW_HEIGHT {
        let t = if PLACEHOLDER_PREVIEW_HEIGHT > 1 {
            y as f32 / (PLACEHOLDER_PREVIEW_HEIGHT - 1) as f32
        } else {
            0.0
        };
        let mut base = lerp_color(top, bottom, t);
        if y >= highlight_start {
            let emphasis = (y - highlight_start) as f32 / 48.0;
            base = blend_toward(base, accent, (0.25 + emphasis * 0.45).clamp(0.0, 0.65));
        }

        for x in 0..PLACEHOLDER_PREVIEW_WIDTH {
            let mut color = base;
            if ((x + y + stripe_offset) % stripe_spacing) == 0 {
                color = blend_toward(color, accent, 0.35);
            }

            let base_noise = noise_seed
                .wrapping_add((x as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15))
                .wrapping_add((y as u64 + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9));
            let noise = (base_noise ^ (base_noise >> 32)) as u8;
            let jitter = (noise as i16 - 128) / 18;
            color = adjust_color_brightness(color, jitter);

            let idx = ((y * PLACEHOLDER_PREVIEW_WIDTH + x) * 4) as usize;
            pixels[idx] = color.r;
            pixels[idx + 1] = color.g;
            pixels[idx + 2] = color.b;
            pixels[idx + 3] = color.a;
        }
    }

    ImageData::new(
        PLACEHOLDER_PREVIEW_WIDTH,
        PLACEHOLDER_PREVIEW_HEIGHT,
        pixels,
    )
}

fn preview_palette(kind: ScenarioKind) -> (Color, Color, Color) {
    match kind {
        ScenarioKind::Scenario => (
            Color::opaque(36, 52, 104),
            Color::opaque(14, 20, 40),
            Color::opaque(220, 184, 104),
        ),
        ScenarioKind::Folder => (
            Color::opaque(30, 68, 72),
            Color::opaque(14, 26, 32),
            Color::opaque(160, 216, 200),
        ),
        ScenarioKind::Editor => (
            Color::opaque(96, 52, 32),
            Color::opaque(32, 20, 16),
            Color::opaque(228, 164, 100),
        ),
    }
}

fn lerp_color(start: Color, end: Color, t: f32) -> Color {
    let clamped = t.clamp(0.0, 1.0);
    let lerp_channel = |s: u8, e: u8| -> u8 {
        (s as f32 + (e as f32 - s as f32) * clamped)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        lerp_channel(start.r, end.r),
        lerp_channel(start.g, end.g),
        lerp_channel(start.b, end.b),
        255,
    )
}

fn blend_toward(base: Color, target: Color, factor: f32) -> Color {
    lerp_color(base, target, factor.clamp(0.0, 1.0))
}

fn adjust_color_brightness(color: Color, delta: i16) -> Color {
    let adjust = |channel: u8| -> u8 {
        let value = channel as i16 + delta;
        value.clamp(0, 255) as u8
    };
    Color::new(adjust(color.r), adjust(color.g), adjust(color.b), color.a)
}

#[derive(Clone, Debug)]
pub(crate) struct FrontendScenario {
    pub(crate) identifier: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) kind: ScenarioKind,
    pub(crate) is_editable: bool,
    pub(crate) is_playable: bool,
    /// Scenario.txt `[Head] MissionAccess`. This is presentation/catalog
    /// metadata only; the live process-local store decides current access.
    pub(crate) mission_access: Option<String>,
    pub(crate) path: Option<PathBuf>,
    /// Every real/logical group path that contributed to this merged entry.
    /// `path` remains the first-root presentation source, while parity
    /// preflights must inspect all contributors.
    pub(crate) source_paths: Vec<PathBuf>,
    pub(crate) root_label: Option<String>,
    pub(crate) preview: Option<ImageData>,
    /// Right-page Title.png/Title.bmp picture (C4ScenarioListLoader::Entry
    /// fctTitle); unlike `preview` this never falls back to Loader/Icon art.
    pub(crate) title_picture: Option<ImageData>,
    pub(crate) children: Vec<FrontendScenario>,
    pub(crate) folder_index: Option<i32>,
    pub(crate) icon_index: Option<i32>,
    pub(crate) difficulty: Option<i32>,
    /// Author of packed groups (C4StartupScenSelDlg.cpp:536-552).
    pub(crate) author: Option<String>,
    /// Version.txt contents (C4StartupScenSelDlg.cpp:554).
    pub(crate) version: Option<String>,
    /// Scenario.txt [Definitions] LocalOnly (C4Scenario.cpp:482).
    pub(crate) local_only: Option<bool>,
    /// Scenario.txt [Definitions] AllowUserChange (C4Scenario.cpp:483).
    pub(crate) allow_user_change: Option<bool>,
    /// Ordered external modules from [Definitions], used to seed the fixed
    /// entries in C4DefinitionSelDlg.
    pub(crate) definition_modules: Vec<String>,
}
