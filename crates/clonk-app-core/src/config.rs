//! Process-local configuration ownership and session compatibility policy.

use clonk_core::std_config::Config;
use clonk_engine::{MissionAccessStore, ShowCommandsRequestStore};

/// Which operating mode the engine runs in.
///
/// Port-only: LegacyClonk has no such switch, because it *is* the thing being
/// reproduced. [`CompatProfile::LegacyClonk`] names the promise written down in
/// `docs/COMPAT_PROFILE.md` and `compat/profile.json` — what this port
/// reproduces from the pinned C++ engine and what it deliberately does not.
///
/// The default is [`CompatProfile::Normal`], and it stays the default. The
/// profile is opt-in because it is a *narrowing*: it forces port-only
/// presentation features off and refuses combinations the promise does not
/// cover, so a player who never asked for it must never be silently placed in
/// it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompatProfile {
    #[default]
    Normal,
    LegacyClonk,
}

impl CompatProfile {
    /// The stored token. `legacy-clonk` is the profile `id` in
    /// `compat/profile.json`, spelled identically so the config value and the
    /// manifest cannot drift apart.
    pub const NORMAL: &'static str = "Normal";
    pub const LEGACY_CLONK: &'static str = "legacy-clonk";

    /// Accepts exactly what `advanced_config`'s enum row accepts: the canonical
    /// token or the index stored alongside it. Anything else is `None`, which
    /// leaves the normal profile in place — an unrecognised value must not
    /// enrol a session in a promise it cannot keep.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            Self::NORMAL | "0" => Some(Self::Normal),
            Self::LEGACY_CLONK | "1" => Some(Self::LegacyClonk),
            _ => None,
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Normal => Self::NORMAL,
            Self::LegacyClonk => Self::LEGACY_CLONK,
        }
    }

    /// What a host/join confirmation shows. `Normal` deliberately reads as an
    /// absence rather than a second named mode, because it promises nothing.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Normal => "No compatibility profile",
            Self::LegacyClonk => "LegacyClonk compatibility",
        }
    }
}

/// Resolve the one profile a session runs under.
///
/// A launch-only override wins over the persisted key and is **never** written
/// back: `--compat-profile` is a property of this run, so a player who launches
/// once in compatibility mode does not find their saved configuration changed
/// afterwards. With no override the persisted value decides, and an
/// unrecognised or absent value is [`CompatProfile::Normal`].
pub fn resolve_compat_profile(
    config: Option<&Config>,
    launch_override: Option<CompatProfile>,
) -> CompatProfile {
    if let Some(profile) = launch_override {
        return profile;
    }
    config
        .and_then(|config| {
            config
                .get_in(Some("General"), "CompatProfile")
                .or_else(|| config.get("CompatProfile"))
        })
        .and_then(CompatProfile::parse)
        .unwrap_or_default()
}

/// `CNM_Decentral`, C++'s `Network.ControlMode` default
/// (`C4GameControlNetwork.h:51`, `C4Config.cpp:540` — "0 is the standard mode
/// set in config").
pub const CPP_CONTROL_MODE_DECENTRAL: i32 = 0;

/// Resolve `Network.ControlMode` for a session that is about to be constructed.
///
/// This is a **non-persistent overlay**, which is the whole point: under the
/// compatibility profile the C++ default wins, and the player's saved
/// `Network.ControlMode` is neither read into the session nor written back. A
/// normal-profile session is untouched and keeps the port's measured async
/// default.
///
/// It is resolved once, at session construction, and the resolved value travels
/// in the prepared host parameters. A configuration edit mid-round therefore
/// cannot move a running session between control modes — which matters because
/// `ControlMode` is synchronized: two peers disagreeing about it is a desync,
/// not a preference.
pub fn session_control_mode(profile: CompatProfile, configured: i32) -> i32 {
    match profile {
        CompatProfile::LegacyClonk => CPP_CONTROL_MODE_DECENTRAL,
        CompatProfile::Normal => configured,
    }
}

/// Resolve the synchronized `FairCrewStrength` parameter for a new session.
///
/// The parameter's own default is 0 (`C4GameParameters.cpp:562`), and C++
/// substitutes the configured strength only once fair crew is actually on:
///
/// ```text
/// if (!FairCrewStrength && UseFairCrew)
///     FairCrewStrength = Config.General.FairCrewStrength;
/// ```
///
/// (`C4GameParameters.cpp:439-440`). It is **synchronized** — compiled into
/// Parameters and published to every peer — so filling it unconditionally
/// advertises a strength stock C++ never sends for an ordinary round. Unlike
/// the control-mode overlay this is not profile-dependent: C++ behaves this
/// way always, so the port should too.
pub fn session_fair_crew_strength(fair_crew: bool, configured: i32) -> i32 {
    if fair_crew {
        configured
    } else {
        0
    }
}

/// Resolve whether the Shared Bases rule may widen base lookup to allied bases.
///
/// A **non-persistent overlay**, like the control-mode one above. `FindBase`
/// drives "back to base" and the wormhole, and `C4Game::FindBase` matches
/// `Base == iPlayer` exactly with no alliance test (`C4Game.cpp:3732-3745`),
/// so the widening is a deliberate divergence and the compatibility profile
/// has to take it away. It is synchronized state — two peers disagreeing about
/// which base a Clonk walks to is a desync, not a preference — so it is
/// resolved once, at session construction, and never read from configuration
/// mid-round.
pub fn session_shared_bases(profile: CompatProfile) -> bool {
    match profile {
        CompatProfile::Normal => true,
        CompatProfile::LegacyClonk => false,
    }
}

/// C++'s in-game application timer: `defaultIngameGameTickDelay`, the literal
/// 28 ms `C4Game::OpenGame` installs once the startup graphics are freed
/// (`C4Game.cpp:63,443`).
pub const CPP_INGAME_GAME_TICK_DELAY_MS: u64 = 28;

/// Resolve the simulation tick cadence for a session that is about to start.
///
/// The port ships the parameterless `SetGameSpeed` cadence — integer
/// `1000 / 38 = 26` ms — because a 28 ms timer caps out at 35.714 updates per
/// wall-clock second and cannot hold Hazard at 38. That is an approved
/// divergence, and it is presentation-only: the cadence never enters savegames,
/// synchronized controls or snapshots, so a fixed frame/control sequence still
/// produces identical state.
///
/// Under the compatibility profile a session runs at C++'s 28 ms anyway, so a
/// recording made here advances in the same wall time as one made natively.
/// Like every other overlay in this module it is resolved at session
/// construction and never written back, leaving an explicit `SetGameSpeed` from
/// script free to retune the timer exactly as it does natively.
pub fn session_game_tick_delay_ms(profile: CompatProfile) -> u64 {
    match profile {
        CompatProfile::LegacyClonk => CPP_INGAME_GAME_TICK_DELAY_MS,
        CompatProfile::Normal => clonk_engine::DEFAULT_GAME_TICK_DELAY_MS,
    }
}

/// C++'s `Network.MaxLoadFileSize` default (`C4Config.cpp:543`).
pub const CPP_MAX_LOAD_FILE_SIZE: u32 = 100 * 1024 * 1024;

/// The normal-profile default is large enough for classic compilation folders
/// that C++'s 100 MiB ceiling leaves non-loadable, while remaining below the
/// signed config field's limit.
pub const DEFAULT_MAX_LOAD_FILE_SIZE: u32 = 256 * 1024 * 1024;

/// Resolve the definition-publication ceiling for a host session.
///
/// A saved value is an explicit host transfer policy and always wins. With no
/// saved value, normal mode uses the port's larger default and the compatibility
/// profile retains the C++ default.
pub fn session_max_load_file_size(profile: CompatProfile, configured: Option<u32>) -> u32 {
    configured.unwrap_or(match profile {
        CompatProfile::Normal => DEFAULT_MAX_LOAD_FILE_SIZE,
        CompatProfile::LegacyClonk => CPP_MAX_LOAD_FILE_SIZE,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeHelpCharset {
    Windows1252,
    Utf8,
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
pub struct ConfigState {
    /// Ordinary runtime config toggles, held until a clean shutdown the way
    /// C++ mutates its process-wide `Config` and saves once in
    /// `C4Application::Clear` (C4Application.cpp:351-367).
    pub deferred: crate::deferred_config::DeferredConfig,
    /// The one operating mode this run resolved to, from the launch override
    /// or the persisted `General.CompatProfile` key
    /// (`resolve_compat_profile`). Held as state rather than
    /// re-read per use so a host and the clients it admits cannot disagree
    /// about it mid-session.
    pub compat_profile: CompatProfile,
    /// Encoding of the process-global resource table. `LoadResStr` reads the
    /// same already-loaded table as the UTF-8 presentation helpers; retain
    /// its source charset so byte-returning call sites do not reopen it.
    pub language_charset: RuntimeHelpCharset,
    /// Process-local Config.General.MissionAccess shared across fresh games.
    pub mission_access: MissionAccessStore,
    /// The mission-access list already on disk, so a round that earns one
    /// writes once rather than on every frame that follows.
    pub persisted_mission_access: String,
    /// `Config.Graphics.ShowFolderMaps`, default-on like C4ConfigGraphics.
    pub show_folder_maps: bool,
    /// Process-local Config.Graphics.ShowCommands enable requests shared
    /// across fresh engines.
    pub show_commands_requests: ShowCommandsRequestStore,
    /// Current `Config.General.GamepadEnabled` value used by each new
    /// `C4Player::InitControl` analogue.
    pub gamepads_enabled: bool,
    /// Startup-time gamepad subsystem gate. Native does not create or later
    /// poll `C4GamePadControl` when this was false during application init.
    pub gamepad_input_enabled: bool,
    pub gamepad_gui_control: bool,
    /// A confirmed `Config.Default()` reset owns shutdown persistence. Keep
    /// this latched after `take_exit_request` so the event-loop tail cannot
    /// merge stale display values back into the freshly reset config.
    pub reset_requested: bool,
}

impl ConfigState {
    /// A scenario-selector or lobby game option, which C++ keeps in its
    /// process-wide `Config` until the shutdown save.
    ///
    /// None of the sites behind these keys writes the file: the runtime-join
    /// toggle is `Config.Network.NoRuntimeJoin = !fAllowed`
    /// (`C4GameOptions.cpp:169`), the league checkbox and remembered password
    /// are plain assignments (`C4Network2Dialogs.cpp:676-686,748`), the control
    /// rate and mode are written from the control layer
    /// (`C4Control.cpp:141`; `C4Network2.cpp:853`), and internet signup and
    /// recording are the `OnBtnInternet`/`OnBtnRecord` toggles already cited on
    /// `deferred_config` (`C4StartupNetDlg.cpp:840-850`). The whole C++ tree
    /// holds seven `Config.Save()` calls and not one of them is a game-option
    /// surface, so an eager write here would keep a change a crash should have
    /// discarded.
    pub fn persist_game_option_value(&mut self, section: &str, key: &str, value: String) {
        self.deferred.set(section, key, value);
    }
    /// The escaped-string form of [`Self::persist_game_option_value`], for a
    /// `CFG_MaxString` field whose flush needs C++'s quoting rather than a raw
    /// scalar (`C4Config.cpp:379`).
    pub fn persist_game_option_text(&mut self, section: &str, key: &str, value: &str) {
        let Some(native) = clonk_resources::encode_legacy_script_text(value) else {
            tracing::warn!(
                section,
                key,
                "game option text is not representable in the classic Windows-1252 config"
            );
            return;
        };
        self.deferred.set_escaped(section, key, value, native);
    }
    pub fn clear_deferred_display_toggles(&mut self) {
        for (section, key) in [
            ("Graphics", "ShowCrewNames"),
            ("Graphics", "ShowCrewCNames"),
            ("Graphics", "ShowClock"),
            ("General", "FPS"),
            ("Graphics", "UpperBoard"),
        ] {
            self.deferred.clear(section, key);
        }
    }
}
