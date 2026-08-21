use clonk_audio::{VoiceInputDeviceId, VoiceProcessingConfig};
use clonk_core::std_config::Config;
use clonk_frontend::startup_options_graphics::{
    MAX_GRAPHICS_SCALE_PERCENT, MIN_GRAPHICS_SCALE_PERCENT,
};
use clonk_platform::AppPaths;
use std::io::ErrorKind;
use winit::keyboard::KeyCode as VirtualKeyCode;

pub(crate) const MAX_VOICE_VOLUME_PERCENT: i32 = 200;

const DEFAULT_MAX_CHANNELS: usize = 1024;
const MAX_CHANNELS_LIMIT: usize = 1024;
// C++ resolution defaults (C4Config.cpp:440-441).
const DEFAULT_RES_X: u32 = 800;
const DEFAULT_RES_Y: u32 = 600;
const MIN_RESOLUTION: u32 = 1;

/// How the microphone is opened once voice chat is enabled at all. Port-only:
/// LegacyClonk has no voice chat, so neither mode has a C++ oracle.
///
/// Push-to-talk is the default and stays the default. A player who has not
/// asked for [`VoiceActivationMode::VoiceActivated`] keeps the property that
/// the microphone is closed unless the configured key is held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VoiceActivationMode {
    #[default]
    PushToTalk,
    VoiceActivated,
}

impl VoiceActivationMode {
    pub const PUSH_TO_TALK: &'static str = "PushToTalk";
    pub const VOICE_ACTIVATED: &'static str = "VoiceActivated";

    /// Accepts exactly what `advanced_config`'s enum row accepts: the canonical
    /// token, case-sensitively, or the index it stores alongside it. Anything
    /// else is `None`, which leaves push-to-talk in place — a typo must not open
    /// a microphone.
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            Self::PUSH_TO_TALK | "0" => Some(Self::PushToTalk),
            Self::VOICE_ACTIVATED | "1" => Some(Self::VoiceActivated),
            _ => None,
        }
    }
}

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

/// The two tuning values a voice-activated capture needs, resolved into the
/// units the gate compares against: a level threshold on the same `0.0..=1.0`
/// scale as [`clonk_audio::voice_activation_level`], and a release tail counted
/// in whole captured frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceActivation {
    pub threshold: f32,
    pub hangover_frames: u32,
}

const VOICE_FRAME_MILLISECONDS: u32 =
    clonk_audio::VOICE_FRAME_SAMPLES as u32 * 1_000 / clonk_audio::VOICE_SAMPLE_RATE;
const MAX_VOICE_ACTIVATION_HANGOVER_MS: i32 = 2_000;

#[derive(Debug, Clone)]
pub struct AudioOptions {
    pub max_channels: usize,
    pub prefer_linear_resampling: bool,
    pub sound_enabled: bool,
    pub music_enabled: bool,
    pub menu_music_enabled: bool,
    pub menu_sound_enabled: bool,
    /// Initial process-local mute state for every client's `/sound` command.
    pub mute_sound_command: bool,
    pub sound_volume: f32,
    pub music_volume: f32,
    /// Port-only, explicit microphone opt-in. LegacyClonk has no voice chat.
    pub voice_enabled: bool,
    /// Remote-playback gain in `0.0..=2.0`; `1.0` is unity and the upper half
    /// boosts quiet speech without changing the music or sound-effect ranges.
    pub voice_volume: f32,
    /// Opaque CPAL microphone endpoint ID. `None` follows the system default
    /// instead of pinning one host-provided endpoint.
    pub voice_input_device: Option<VoiceInputDeviceId>,
    pub voice_push_to_talk: VirtualKeyCode,
    pub voice_activation_mode: VoiceActivationMode,
    /// Level a captured frame must reach to open a voice-activated capture, on
    /// the `0.0..=1.0` scale of [`clonk_audio::voice_activation_level`]. `0.0`
    /// transmits everything the capture hears; `1.0` never opens.
    pub voice_activation_threshold: f32,
    /// How long a voice-activated capture keeps transmitting after the level
    /// falls back below the threshold, so word endings are not clipped.
    pub voice_activation_hangover_ms: u32,
    /// Subtract what this machine is playing from what its microphone hears,
    /// so speakers do not send the game — or the other players — back out.
    pub voice_echo_cancellation: bool,
    /// Hold down the part of the capture that does not change while someone
    /// talks: fans, hum, traffic.
    pub voice_noise_suppression: bool,
    /// Bring every talker to the same loudness, whatever their microphone and
    /// however far from it they sit.
    pub voice_automatic_gain_control: bool,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            max_channels: DEFAULT_MAX_CHANNELS,
            prefer_linear_resampling: false,
            sound_enabled: true,
            music_enabled: true,
            menu_music_enabled: true,
            menu_sound_enabled: true,
            mute_sound_command: false,
            sound_volume: 1.0,
            music_volume: 1.0,
            voice_enabled: false,
            voice_volume: 1.0,
            voice_input_device: None,
            voice_push_to_talk: VirtualKeyCode::Backquote,
            voice_activation_mode: VoiceActivationMode::default(),
            // -36 dBFS: above a quiet room, below ordinary speech.
            voice_activation_threshold: 0.4,
            voice_activation_hangover_ms: 400,
            // Capture processing follows the microphone opt-in rather than
            // being a second one: a player who has enabled voice chat wants to
            // be heard, not to send their room back to everyone else
            // (clonk-org/clonk-rs#421).
            voice_echo_cancellation: true,
            voice_noise_suppression: true,
            voice_automatic_gain_control: true,
        }
    }
}

impl AudioOptions {
    /// The audio a `USE_CONSOLE` build has: none. `ENABLE_SOUND` is a
    /// dependent option forced OFF for that build (CMakeLists.txt:183-185), so
    /// a dedicated server never opens a device — the configured volumes and
    /// channel count are irrelevant behind four disabled flags.
    pub fn silenced() -> Self {
        Self {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..Self::default()
        }
    }

    pub fn load(paths: Option<&AppPaths>) -> Self {
        let mut options = Self::default();
        let Some(paths) = paths else {
            return options;
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => options.apply_config(&config),
            Err(err) => {
                if err.kind() != ErrorKind::NotFound {
                    tracing::warn!(
                        error = %err,
                        path = %config_path.display(),
                        "failed to load audio config"
                    );
                }
            }
        }
        options
    }

    fn apply_config(&mut self, config: &Config) {
        if let Some(raw) = config.get_in(Some("Sound"), "Sound") {
            if let Some(parsed) = parse_bool(raw) {
                self.sound_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "Music") {
            if let Some(parsed) = parse_bool(raw) {
                self.music_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MenuMusic") {
            if let Some(parsed) = parse_bool(raw) {
                self.menu_music_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MenuSound") {
            if let Some(parsed) = parse_bool(raw) {
                self.menu_sound_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MuteSoundCommand") {
            if let Some(parsed) = parse_bool(raw) {
                self.mute_sound_command = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "PreferLinearResampling") {
            if let Some(parsed) = parse_bool(raw) {
                self.prefer_linear_resampling = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MusicVolume") {
            if let Some(value) = parse_native_config_integer(raw) {
                let clamped = value.clamp(0, 100);
                self.music_volume = clamped as f32 / 100.0;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "SoundVolume") {
            if let Some(value) = parse_native_config_integer(raw) {
                let clamped = value.clamp(0, 100);
                self.sound_volume = clamped as f32 / 100.0;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MaxChannels") {
            if let Some(value) = parse_native_config_integer(raw) {
                let clamped = value.clamp(1, MAX_CHANNELS_LIMIT as i32);
                self.max_channels = clamped as usize;
            }
        }

        if let Some(raw) = config.get_in(Some("Voice"), "Enabled") {
            if let Some(parsed) = parse_bool(raw) {
                self.voice_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Voice"), "Volume") {
            if let Some(value) = parse_native_config_integer(raw) {
                self.set_voice_volume_percent(value);
            }
        }

        if let Some(raw) = config.get_in(Some("Voice"), "InputDevice") {
            self.voice_input_device = raw.parse().ok();
        }

        if let Some(raw) = config.get_in(Some("Voice"), "PushToTalkKey") {
            if let Some(key) =
                parse_native_config_integer(raw).and_then(crate::input::decode_platform_key_code)
            {
                self.voice_push_to_talk = key;
            }
        }

        // An unreadable mode deliberately leaves the push-to-talk default in
        // place rather than falling through to an open microphone.
        if let Some(mode) = config
            .get_in(Some("Voice"), "ActivationMode")
            .and_then(VoiceActivationMode::parse)
        {
            self.voice_activation_mode = mode;
        }

        if let Some(raw) = config.get_in(Some("Voice"), "ActivationThreshold") {
            if let Some(value) = parse_native_config_integer(raw) {
                self.voice_activation_threshold = value.clamp(0, 100) as f32 / 100.0;
            }
        }

        if let Some(raw) = config.get_in(Some("Voice"), "ActivationHangover") {
            if let Some(value) = parse_native_config_integer(raw) {
                self.voice_activation_hangover_ms =
                    value.clamp(0, MAX_VOICE_ACTIVATION_HANGOVER_MS) as u32;
            }
        }

        for (key, target) in [
            ("EchoCancellation", &mut self.voice_echo_cancellation),
            ("NoiseSuppression", &mut self.voice_noise_suppression),
            (
                "AutomaticGainControl",
                &mut self.voice_automatic_gain_control,
            ),
        ] {
            if let Some(parsed) = config.get_in(Some("Voice"), key).and_then(parse_bool) {
                *target = parsed;
            }
        }
    }

    /// Which capture-processing stages the microphone runs
    /// (clonk-org/clonk-rs#421). Independent of how the microphone opens: both
    /// push-to-talk and voice activation capture through the same chain.
    pub(crate) fn voice_processing(&self) -> VoiceProcessingConfig {
        VoiceProcessingConfig {
            echo_cancellation: self.voice_echo_cancellation,
            noise_suppression: self.voice_noise_suppression,
            automatic_gain_control: self.voice_automatic_gain_control,
        }
    }

    /// The gate settings for a voice-activated capture, or `None` while the
    /// player is on the push-to-talk default and every captured frame goes out.
    pub(crate) fn voice_activation(&self) -> Option<VoiceActivation> {
        (self.voice_activation_mode == VoiceActivationMode::VoiceActivated).then(|| {
            VoiceActivation {
                threshold: self.voice_activation_threshold,
                hangover_frames: self
                    .voice_activation_hangover_ms
                    .div_ceil(VOICE_FRAME_MILLISECONDS),
            }
        })
    }

    pub(crate) fn music_volume_percent(&self) -> i32 {
        normalized_volume_percent(self.music_volume, 100)
    }

    pub(crate) fn sound_volume_percent(&self) -> i32 {
        normalized_volume_percent(self.sound_volume, 100)
    }

    /// `Config.Voice.Volume`, in the `0..=200` domain the Audio sheet's bar and
    /// the Advanced editor's row both use. `100` is unity gain.
    pub(crate) fn voice_volume_percent(&self) -> i32 {
        normalized_volume_percent(self.voice_volume, MAX_VOICE_VOLUME_PERCENT)
    }

    pub(crate) fn set_music_volume_percent(&mut self, value: i32) {
        self.music_volume = normalized_volume(value, 100);
    }

    pub(crate) fn set_sound_volume_percent(&mut self, value: i32) {
        self.sound_volume = normalized_volume(value, 100);
    }

    pub(crate) fn set_voice_volume_percent(&mut self, value: i32) {
        self.voice_volume = normalized_volume(value, MAX_VOICE_VOLUME_PERCENT);
    }

    /// Writes exactly the six values owned by the classic startup Sound
    /// sheet. `MaxChannels` and all unknown/extension keys deliberately stay
    /// untouched when the surrounding config is saved.
    pub(crate) fn write_startup_sound_config(&self, config: &mut Config) {
        let section = Some("Sound");
        config.set_in(section, "Sound", bool_config_value(self.sound_enabled));
        config.set_in(section, "Music", bool_config_value(self.music_enabled));
        config.set_in(
            section,
            "MenuMusic",
            bool_config_value(self.menu_music_enabled),
        );
        config.set_in(
            section,
            "MenuSound",
            bool_config_value(self.menu_sound_enabled),
        );
        config.set_in(
            section,
            "MusicVolume",
            self.music_volume_percent().to_string(),
        );
        config.set_in(
            section,
            "SoundVolume",
            self.sound_volume_percent().to_string(),
        );
    }

    /// The port-only `[Voice]` keys the Audio sheet edits
    /// (clonk-org/clonk-rs#452). They live in their own section rather than
    /// `[Sound]` because `[Sound]` is C4Config's on-disk layout and every key
    /// in it has a C++ counterpart.
    pub(crate) fn write_startup_voice_config(&self, config: &mut Config) {
        let section = Some("Voice");
        config.set_in(section, "Enabled", bool_config_value(self.voice_enabled));
        config.set_in(section, "Volume", self.voice_volume_percent().to_string());
        config.set_in(
            section,
            "InputDevice",
            self.voice_input_device
                .as_ref()
                .map_or("", VoiceInputDeviceId::as_str),
        );
        if let Some(encoded) = crate::input::encode_virtual_key_code(self.voice_push_to_talk) {
            config.set_in(section, "PushToTalkKey", encoded.to_string());
        }
        // The canonical token, never a display string: `parse` is
        // case-sensitive and falls back to push-to-talk, so a localized label
        // written here would silently revert the player's choice.
        config.set_in(
            section,
            "ActivationMode",
            match self.voice_activation_mode {
                VoiceActivationMode::PushToTalk => VoiceActivationMode::PUSH_TO_TALK,
                VoiceActivationMode::VoiceActivated => VoiceActivationMode::VOICE_ACTIVATED,
            },
        );
    }
}

fn normalized_volume(value: i32, maximum_percent: i32) -> f32 {
    value.clamp(0, maximum_percent) as f32 / 100.0
}

fn normalized_volume_percent(value: f32, maximum_percent: i32) -> i32 {
    if value.is_finite() {
        ((value * 100.0).round() as i32).clamp(0, maximum_percent)
    } else {
        0
    }
}

fn bool_config_value(value: bool) -> &'static str {
    // StdCompilerINIWrite::Boolean emits these exact spellings
    // (StdCompiler.cpp:345-349).
    if value {
        "true"
    } else {
        "false"
    }
}

/// `StdCompilerINIRead::Boolean` (StdCompiler.cpp:692-715). C++ reads the raw
/// value in place: a leading `1`/`0` not followed by another digit, or a
/// case-sensitive `true`/`false` prefix. No trimming and no case folding, so
/// `TRUE`, ` 1` and `10` are all not-found and leave the adapted default.
/// `StdCompilerINIRead::ReadNum` (StdCompiler.h:705-724): skip whitespace,
/// select base 16 only for a leading `0x`/`0X`, then consume the longest valid
/// numeric prefix and ignore whatever follows. No digits means not-found, so
/// the caller keeps the field's adapted default.
fn parse_native_config_integer(raw: &str) -> Option<i32> {
    crate::parse_startup_config_integer(raw.as_bytes())
}

fn parse_bool(raw: &str) -> Option<bool> {
    let value = raw.as_bytes();
    if value.first() == Some(&b'1') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if value.first() == Some(&b'0') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if value.starts_with(b"true") {
        Some(true)
    } else if value.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Window,
    Fullscreen,
}

impl DisplayMode {
    fn from_config(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "1" | "window" | "windowed" => Some(DisplayMode::Window),
            "0" | "fullscreen" => Some(DisplayMode::Fullscreen),
            value => value.parse::<i32>().ok().and_then(|parsed| match parsed {
                0 => Some(DisplayMode::Fullscreen),
                1 => Some(DisplayMode::Window),
                _ => None,
            }),
        }
    }

    fn to_config_value(self) -> &'static str {
        match self {
            DisplayMode::Window => "1",
            DisplayMode::Fullscreen => "0",
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn silenced_audio_options_open_no_device_like_a_console_build() {
        // `ENABLE_SOUND` is a dependent option forced OFF whenever
        // `USE_CONSOLE` is set (CMakeLists.txt:183-185), so a dedicated server
        // has no sound support compiled in at all — neither in-game nor menu.
        let silenced = AudioOptions::silenced();
        assert!(!silenced.sound_enabled);
        assert!(!silenced.music_enabled);
        assert!(!silenced.menu_sound_enabled);
        assert!(!silenced.menu_music_enabled);
    }

    #[test]
    fn audio_options_default_channel_count_matches_cpp() {
        // C4AudioSystem::MaxChannels is 1024, and C4ConfigSound::CompileFunc
        // uses it as the default (C4AudioSystem.h:103; C4Config.cpp:516).
        assert_eq!(AudioOptions::default().max_channels, 1024);
    }

    #[test]
    fn audio_options_read_prefer_linear_resampling() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "PreferLinearResampling", "true");
        let mut options = AudioOptions::default();

        options.apply_config(&config);

        assert!(options.prefer_linear_resampling);
    }

    #[test]
    fn audio_options_load_sound_command_mute_without_owning_its_persistence() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "MuteSoundCommand", "true");
        let mut options = AudioOptions::default();
        options.apply_config(&config);
        assert!(options.mute_sound_command);
    }

    #[test]
    fn audio_options_sound_sheet_percent_accessors_clamp_and_round() {
        let mut options = AudioOptions::default();
        options.set_music_volume_percent(-1);
        options.set_sound_volume_percent(101);
        assert_eq!(options.music_volume_percent(), 0);
        assert_eq!(options.sound_volume_percent(), 100);

        options.music_volume = 0.555;
        options.sound_volume = f32::NAN;
        assert_eq!(options.music_volume_percent(), 56);
        assert_eq!(options.sound_volume_percent(), 0);
    }

    #[test]
    fn voice_volume_percent_supports_boost_while_classic_volumes_stay_bounded() {
        let options = AudioOptions {
            music_volume: 1.75,
            sound_volume: 1.75,
            voice_volume: 1.75,
            ..AudioOptions::default()
        };

        assert_eq!(options.music_volume_percent(), 100);
        assert_eq!(options.sound_volume_percent(), 100);
        assert_eq!(options.voice_volume_percent(), 175);
    }

    #[test]
    fn audio_options_write_only_the_six_classic_sound_sheet_keys() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "MaxChannels", "37");
        config.set_in(Some("Sound"), "VendorExtension", "keep-me");
        config.set_in(Some("General"), "Unrelated", "also-keep-me");
        let options = AudioOptions {
            max_channels: 999,
            prefer_linear_resampling: true,
            sound_enabled: false,
            music_enabled: true,
            menu_music_enabled: false,
            menu_sound_enabled: true,
            mute_sound_command: true,
            sound_volume: 0.27,
            music_volume: 0.83,
            voice_enabled: true,
            voice_volume: 0.42,
            voice_input_device: None,
            voice_push_to_talk: VirtualKeyCode::KeyT,
            voice_activation_mode: VoiceActivationMode::VoiceActivated,
            voice_activation_threshold: 0.25,
            voice_activation_hangover_ms: 260,
            voice_echo_cancellation: false,
            voice_noise_suppression: true,
            voice_automatic_gain_control: false,
        };

        options.write_startup_sound_config(&mut config);

        assert_eq!(config.get_in(Some("Sound"), "Sound"), Some("false"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("true"));
        assert_eq!(config.get_in(Some("Sound"), "MenuMusic"), Some("false"));
        assert_eq!(config.get_in(Some("Sound"), "MenuSound"), Some("true"));
        assert_eq!(config.get_in(Some("Sound"), "MusicVolume"), Some("83"));
        assert_eq!(config.get_in(Some("Sound"), "SoundVolume"), Some("27"));
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("37"));
        assert_eq!(
            config.get_in(Some("Sound"), "MuteSoundCommand"),
            None,
            "the startup sound sheet does not own the /sound mute setting"
        );
        assert_eq!(
            config.get_in(Some("Sound"), "VendorExtension"),
            Some("keep-me")
        );
        assert_eq!(
            config.get_in(Some("General"), "Unrelated"),
            Some("also-keep-me")
        );
    }

    /// The Audio sheet's port-only row round-trips through `[Voice]` and never
    /// reaches `[Sound]`, whose keys all have a C4Config counterpart
    /// (clonk-org/clonk-rs#452).
    #[test]
    fn audio_options_write_only_the_port_only_voice_keys() {
        let mut config = Config::new();
        config.set_in(Some("Voice"), "VendorExtension", "keep-me");
        let options = AudioOptions {
            voice_enabled: true,
            voice_volume: 0.42,
            voice_push_to_talk: VirtualKeyCode::KeyT,
            voice_activation_mode: VoiceActivationMode::VoiceActivated,
            ..AudioOptions::default()
        };

        options.write_startup_voice_config(&mut config);

        assert_eq!(config.get_in(Some("Voice"), "Enabled"), Some("true"));
        assert_eq!(config.get_in(Some("Voice"), "Volume"), Some("42"));
        let expected_key = crate::input::encode_virtual_key_code(VirtualKeyCode::KeyT)
            .expect("T has a native key code")
            .to_string();
        assert_eq!(
            config.get_in(Some("Voice"), "PushToTalkKey"),
            Some(expected_key.as_str())
        );
        assert_eq!(
            config.get_in(Some("Voice"), "ActivationMode"),
            Some(VoiceActivationMode::VOICE_ACTIVATED),
            "the canonical token, never a display string",
        );
        assert_eq!(
            config.get_in(Some("Voice"), "VendorExtension"),
            Some("keep-me")
        );
        // The tuning values stay in the Advanced editor: the Audio sheet has
        // no room for them and they are set-once, not a choice.
        assert_eq!(config.get_in(Some("Voice"), "ActivationThreshold"), None);
        assert_eq!(config.get_in(Some("Voice"), "ActivationHangover"), None);
        assert_eq!(config.get_in(Some("Voice"), "EchoCancellation"), None);
        assert_eq!(config.get_in(Some("Voice"), "NoiseSuppression"), None);
        assert_eq!(config.get_in(Some("Voice"), "AutomaticGainControl"), None);
        assert_eq!(
            config.get_in(Some("Sound"), "Sound"),
            None,
            "the port-only row must not reach the classic sound section"
        );

        // The written values are exactly what `apply_config` reads back.
        let mut reloaded = AudioOptions::default();
        reloaded.apply_config(&config);
        assert!(reloaded.voice_enabled);
        assert_eq!(reloaded.voice_volume, 0.42);
        assert_eq!(reloaded.voice_push_to_talk, VirtualKeyCode::KeyT);
        assert_eq!(
            reloaded.voice_activation_mode,
            VoiceActivationMode::VoiceActivated
        );
    }

    #[test]
    fn voice_input_device_id_round_trips_through_config_serialization() {
        let selected = r#"coreaudio:Built-in "Mic"\USB"#
            .parse()
            .expect("valid host-qualified device ID");
        let options = AudioOptions {
            voice_input_device: Some(selected),
            ..AudioOptions::default()
        };
        let mut config = Config::new();

        options.write_startup_voice_config(&mut config);
        let serialized = config.to_string().expect("config serializes");
        let mut reader = serialized.as_bytes();
        let reloaded_config = Config::from_reader(&mut reader).expect("config reloads");
        let mut reloaded = AudioOptions::default();
        reloaded.apply_config(&reloaded_config);

        assert_eq!(reloaded.voice_input_device, options.voice_input_device);
    }

    #[test]
    fn malformed_nonempty_voice_input_identity_never_falls_back_to_the_default_microphone() {
        let mut config = Config::new();
        config.set_in(Some("Voice"), "InputDevice", "corrupt persisted identity");

        let mut loaded = AudioOptions::default();
        loaded.apply_config(&config);

        assert_eq!(
            loaded
                .voice_input_device
                .as_ref()
                .map(VoiceInputDeviceId::as_str),
            Some("corrupt persisted identity"),
            "only an explicitly empty selection may mean system default",
        );
    }

    #[test]
    fn default_voice_input_device_clears_previous_explicit_id() {
        let mut config = Config::new();
        config.set_in(Some("Voice"), "InputDevice", "coreaudio:old-device");

        AudioOptions::default().write_startup_voice_config(&mut config);

        assert_eq!(config.get_in(Some("Voice"), "InputDevice"), Some(""));
        let mut reloaded = AudioOptions::default();
        reloaded.apply_config(&config);
        assert_eq!(reloaded.voice_input_device, None);
    }

    #[test]
    fn voice_options_are_opt_in_and_load_from_the_port_only_section() {
        let defaults = AudioOptions::default();
        assert!(!defaults.voice_enabled);
        assert_eq!(defaults.voice_volume, 1.0);
        assert_eq!(defaults.voice_push_to_talk, VirtualKeyCode::Backquote);

        let mut config = Config::new();
        config.set_in(Some("Voice"), "Enabled", "true");
        config.set_in(Some("Voice"), "Volume", "37");
        config.set_in(
            Some("Voice"),
            "PushToTalkKey",
            crate::input::encode_virtual_key_code(VirtualKeyCode::KeyT)
                .expect("T has a native key code")
                .to_string(),
        );
        let mut options = AudioOptions::default();
        options.apply_config(&config);

        assert!(options.voice_enabled);
        assert_eq!(options.voice_volume, 0.37);
        assert_eq!(options.voice_push_to_talk, VirtualKeyCode::KeyT);
    }

    #[test]
    fn voice_volume_config_round_trips_boost_and_clamps_above_200_percent() {
        let mut boosted_config = Config::new();
        boosted_config.set_in(Some("Voice"), "Volume", "175");
        let mut boosted = AudioOptions::default();
        boosted.apply_config(&boosted_config);

        assert_eq!(boosted.voice_volume, 1.75);
        assert_eq!(boosted.voice_volume_percent(), 175);
        boosted.write_startup_voice_config(&mut boosted_config);
        assert_eq!(boosted_config.get_in(Some("Voice"), "Volume"), Some("175"));

        let mut excessive_config = Config::new();
        excessive_config.set_in(Some("Voice"), "Volume", "250");
        let mut excessive = AudioOptions::default();
        excessive.apply_config(&excessive_config);

        assert_eq!(excessive.voice_volume, 2.0);
        assert_eq!(excessive.voice_volume_percent(), 200);
        excessive.write_startup_voice_config(&mut excessive_config);
        assert_eq!(
            excessive_config.get_in(Some("Voice"), "Volume"),
            Some("200"),
        );
    }

    #[test]
    fn capture_processing_is_on_by_default_and_each_stage_switches_off_alone() {
        let defaults = AudioOptions::default();
        assert_eq!(
            defaults.voice_processing(),
            VoiceProcessingConfig::default(),
            "an opted-in microphone is cleaned up unless the player says otherwise",
        );

        let mut config = Config::new();
        config.set_in(Some("Voice"), "NoiseSuppression", "false");
        let mut options = AudioOptions::default();
        options.apply_config(&config);

        assert_eq!(
            options.voice_processing(),
            VoiceProcessingConfig {
                noise_suppression: false,
                ..VoiceProcessingConfig::default()
            },
            "switching one stage off leaves the other two running",
        );

        config.set_in(Some("Voice"), "EchoCancellation", "0");
        config.set_in(Some("Voice"), "AutomaticGainControl", "nonsense");
        let mut options = AudioOptions::default();
        options.apply_config(&config);

        assert_eq!(
            options.voice_processing(),
            VoiceProcessingConfig {
                echo_cancellation: false,
                noise_suppression: false,
                automatic_gain_control: true,
            },
            "an unreadable value leaves its stage as it was, like every other Voice key",
        );
    }

    #[test]
    fn voice_activation_is_an_opt_in_alternative_to_the_default_push_to_talk() {
        let defaults = AudioOptions::default();
        assert_eq!(
            defaults.voice_activation_mode,
            VoiceActivationMode::PushToTalk,
            "the microphone stays key-held unless the player asks for otherwise",
        );
        assert_eq!(defaults.voice_activation_threshold, 0.4);
        assert_eq!(defaults.voice_activation_hangover_ms, 400);
        assert!(defaults.voice_activation().is_none());

        let mut config = Config::new();
        config.set_in(Some("Voice"), "ActivationMode", "VoiceActivated");
        config.set_in(Some("Voice"), "ActivationThreshold", "25");
        config.set_in(Some("Voice"), "ActivationHangover", "260");
        let mut options = AudioOptions::default();
        options.apply_config(&config);

        assert_eq!(
            options.voice_activation_mode,
            VoiceActivationMode::VoiceActivated
        );
        assert_eq!(options.voice_activation_threshold, 0.25);
        assert_eq!(options.voice_activation_hangover_ms, 260);
        let activation = options
            .voice_activation()
            .expect("voice activation is configured");
        assert_eq!(activation.threshold, 0.25);
        assert_eq!(
            activation.hangover_frames, 13,
            "260 ms rounds up to 13 whole 20 ms frames of tail",
        );
    }

    #[test]
    fn voice_activation_config_rejects_values_outside_its_documented_range() {
        let mut config = Config::new();
        config.set_in(Some("Voice"), "ActivationMode", "Nonsense");
        config.set_in(Some("Voice"), "ActivationThreshold", "175");
        config.set_in(Some("Voice"), "ActivationHangover", "-1");
        let mut options = AudioOptions::default();
        options.apply_config(&config);

        assert_eq!(
            options.voice_activation_mode,
            VoiceActivationMode::PushToTalk,
            "an unreadable mode must not silently open the microphone",
        );
        assert_eq!(options.voice_activation_threshold, 1.0);
        assert_eq!(options.voice_activation_hangover_ms, 0);
    }

    #[test]
    fn voice_activation_mode_reads_exactly_what_the_advanced_editor_writes() {
        // `advanced_config::enum_row` renders whichever spelling it accepts, so
        // a value this parser rejects but the editor renders would show one
        // mode in the dialog while the microphone obeys the other — and saving
        // the dialog would then silently rewrite the player's choice.
        let read = |raw: &str| {
            let mut config = Config::new();
            config.set_in(Some("Voice"), "ActivationMode", raw);
            let mut options = AudioOptions::default();
            options.apply_config(&config);
            options.voice_activation_mode
        };

        assert_eq!(
            read(" VoiceActivated "),
            VoiceActivationMode::VoiceActivated
        );
        assert_eq!(
            read("1"),
            VoiceActivationMode::VoiceActivated,
            "the editor also accepts the enum's index",
        );
        assert_eq!(read("0"), VoiceActivationMode::PushToTalk);
        assert_eq!(
            read("voiceactivated"),
            VoiceActivationMode::PushToTalk,
            "the editor matches the token case-sensitively, so this parser must too",
        );
    }

    #[test]
    fn display_options_apply_config_parses_values() {
        // The config file is shared with the C++ engine, which names the
        // keys ResolutionX/ResolutionY (C4Config.cpp:440-441).
        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "ResolutionX", "1280");
        cfg.set_in(Some("Graphics"), "ResolutionY", "720");
        cfg.set_in(Some("Graphics"), "Scale", "150");
        cfg.set_in(Some("Graphics"), "PointFiltering", "true");
        cfg.set_in(Some("Graphics"), "DisplayMode", "0");
        cfg.set_in(Some("Graphics"), "Maximized", "true");
        cfg.set_in(Some("Graphics"), "PositionX", "42");
        cfg.set_in(Some("Graphics"), "PositionY", "84");

        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);

        assert_eq!(options.base_width, 1280);
        assert_eq!(options.base_height, 720);
        assert!((options.scale - 1.5).abs() < f32::EPSILON);
        assert!(options.point_filtering);
        assert_eq!(options.mode, DisplayMode::Fullscreen);
        assert!(options.maximized);
        assert_eq!(options.position, Some((42, 84)));
    }

    #[test]
    fn display_options_default_resolution_matches_cpp() {
        // C4Config.cpp:440-441 defaults ResolutionX/Y to 800x600.
        let options = DisplayOptions::default();
        assert_eq!(options.base_width, 800);
        assert_eq!(options.base_height, 600);
    }

    #[test]
    fn first_run_display_scale_follows_the_monitor_density_only_without_a_config() {
        // Deliberate divergence: C++ always starts at Scale=100, so a 2x
        // panel gets an 800x600 *device pixel* window with a 14px font
        // (src/C4Config.cpp:440-441, :480). Seeding the application scale
        // from the monitor keeps the classic 800x600 logical layout while
        // giving it the panel's real pixel density — the window covers the
        // same physical area it did on a 1x display.
        let mut fresh = DisplayOptions::default();
        fresh.mark_first_run();
        assert!(fresh.apply_first_run_display_scale(2.0));
        assert_eq!(fresh.scale_percent(), 200);
        assert_eq!(fresh.base_width, 800, "the logical layout is unchanged");
        assert_eq!(fresh.base_height, 600);
        assert_eq!(fresh.actual_size(), (1600, 1200));

        // A configuration that exists on disk is the player's choice.
        let mut configured = DisplayOptions::default();
        assert!(!configured.apply_first_run_display_scale(2.0));
        assert_eq!(configured.scale_percent(), 100);

        // Fractional densities round to an integer scale: a non-integer
        // application scale routes every glyph through a bilinear resample
        // of the atlas (`requires_resampling`, clonk_fonts.rs:102-113).
        for (factor, percent) in [(1.0, 100), (1.25, 100), (1.5, 200), (2.0, 200), (3.0, 300)] {
            let mut options = DisplayOptions::default();
            options.mark_first_run();
            options.apply_first_run_display_scale(factor);
            assert_eq!(options.scale_percent(), percent, "scale factor {factor}");
        }

        // Beyond the supported range the scale clamps rather than producing
        // an unreachable Options-dialog value.
        let mut huge = DisplayOptions::default();
        huge.mark_first_run();
        huge.apply_first_run_display_scale(9.0);
        assert_eq!(huge.scale_percent(), MAX_GRAPHICS_SCALE_PERCENT);

        // Degenerate factors never disturb the default.
        for factor in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let mut options = DisplayOptions::default();
            options.mark_first_run();
            assert!(!options.apply_first_run_display_scale(factor));
            assert_eq!(options.scale_percent(), 100);
        }
    }

    #[test]
    fn display_mode_default_override_and_numeric_persistence_match_cpp() {
        let mut missing_mode = DisplayOptions::default();
        assert_eq!(missing_mode.mode, DisplayMode::Fullscreen);
        missing_mode.apply_config(&Config::new());
        assert!(
            missing_mode.dirty,
            "an absent DisplayMode is materialized by the shutdown save"
        );
        let mut persisted = Config::new();
        missing_mode.write_config(&mut persisted);
        assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode"), Some("0"));

        for raw in ["1", "Window"] {
            let mut config = Config::new();
            config.set_in(Some("Graphics"), "DisplayMode", raw);
            let mut options = DisplayOptions::default();
            options.apply_config(&config);
            assert_eq!(options.mode, DisplayMode::Window, "raw value {raw}");
            assert!(!options.dirty, "an explicit valid mode stays clean");

            let mut persisted = Config::new();
            options.write_config(&mut persisted);
            assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode"), Some("1"));
        }
    }

    #[test]
    fn display_options_preserve_unclamped_cpp_scale_percent() {
        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "Scale", "500");
        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);
        assert!((options.scale - 5.0).abs() < f32::EPSILON);
        assert_eq!(options.checked_loader_actual_size(), Ok((4000, 3000)));
    }

    #[test]
    fn scale_fifty_uses_half_size_and_survives_resize_round_trip() {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), "ResolutionX", "800");
        config.set_in(Some("Graphics"), "ResolutionY", "600");
        config.set_in(Some("Graphics"), "Scale", "50");

        let mut options = DisplayOptions::default();
        options.apply_config(&config);
        assert_eq!(options.scale_percent(), 50);
        assert_eq!(options.scale, 0.5);
        assert_eq!(options.actual_size(), (400, 300));
        assert_eq!(options.checked_loader_actual_size(), Ok((400, 300)));

        options.record_actual_size(401, 301);
        assert_eq!((options.base_width, options.base_height), (802, 602));
        assert_eq!(options.scale_percent(), 50);

        let mut persisted = Config::new();
        options.write_config(&mut persisted);
        assert_eq!(persisted.get_in(Some("Graphics"), "Scale"), Some("50"));

        let mut reloaded = DisplayOptions::default();
        reloaded.apply_config(&persisted);
        assert_eq!(reloaded.scale_percent(), 50);
        assert_eq!(reloaded.actual_size(), (401, 301));
    }

    #[test]
    fn loader_scale_validation_accepts_fractional_and_rejects_nonpositive_or_overflow() {
        for percent in [0, -100] {
            let mut cfg = Config::new();
            cfg.set_in(Some("Graphics"), "Scale", percent.to_string());
            let mut options = DisplayOptions::default();
            options.apply_config(&cfg);
            assert!(options.checked_loader_actual_size().is_err());
        }

        let mut fractional = Config::new();
        fractional.set_in(Some("Graphics"), "ResolutionX", "320");
        fractional.set_in(Some("Graphics"), "ResolutionY", "200");
        fractional.set_in(Some("Graphics"), "Scale", "150");
        let mut options = DisplayOptions::default();
        options.apply_config(&fractional);
        assert_eq!(options.checked_loader_actual_size(), Ok((480, 300)));

        let mut tiny = Config::new();
        tiny.set_in(Some("Graphics"), "ResolutionX", "1");
        tiny.set_in(Some("Graphics"), "ResolutionY", "1");
        tiny.set_in(Some("Graphics"), "Scale", "1");
        let mut options = DisplayOptions::default();
        options.apply_config(&tiny);
        assert_eq!(options.checked_loader_actual_size(), Ok((1, 1)));

        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "ResolutionX", i32::MAX.to_string());
        cfg.set_in(Some("Graphics"), "Scale", "300");
        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);
        assert!(options
            .checked_loader_actual_size()
            .expect_err("output width must be checked")
            .contains("overflows"));
    }

    #[test]
    fn display_options_persist_writes_cpp_key_names() {
        let mut cfg = Config::new();
        let options = DisplayOptions {
            base_width: 1371,
            base_height: 858,
            scale: 3.0,
            scale_percent: 300,
            point_filtering: true,
            mode: DisplayMode::Window,
            maximized: false,
            position: None,
            dirty: true,
            first_run: false,
        };
        options.write_config(&mut cfg);
        assert_eq!(cfg.get_in(Some("Graphics"), "ResolutionX"), Some("1371"));
        assert_eq!(cfg.get_in(Some("Graphics"), "ResolutionY"), Some("858"));
        assert_eq!(cfg.get_in(Some("Graphics"), "Scale"), Some("300"));
        assert_eq!(cfg.get_in(Some("Graphics"), "PointFiltering"), Some("true"));
    }

    #[test]
    fn display_options_record_actual_size_updates_base_resolution() {
        let mut options = DisplayOptions {
            base_width: 800,
            base_height: 600,
            mode: DisplayMode::Window,
            ..Default::default()
        };
        options.record_actual_size(1024, 768);
        assert_eq!(options.base_width, 1024);
        assert_eq!(options.base_height, 768);
        assert!(options.dirty);
        options.dirty = false;
        // Should not update when values identical
        options.record_actual_size(1024, 768);
        assert!(!options.dirty);
    }

    #[test]
    fn display_options_record_actual_size_divides_by_scale_with_ceil() {
        // C4Application::SetResolution stores ceil(pixels / scale)
        // (C4Application.cpp:536-538).
        let mut options = DisplayOptions {
            base_width: 800,
            base_height: 600,
            scale: 3.0,
            scale_percent: 300,
            mode: DisplayMode::Window,
            ..Default::default()
        };
        options.record_actual_size(2743, 1717);
        assert_eq!(options.base_width, 915);
        assert_eq!(options.base_height, 573);
    }

    #[test]
    fn accepted_scale_retains_physical_size_and_recomputes_base_resolution() {
        let mut options = DisplayOptions::default();
        options.record_scale_percent(200, 1_280, 720);
        assert_eq!(options.scale_percent(), 200);
        assert!((options.scale - 2.0).abs() < f32::EPSILON);
        assert_eq!((options.base_width, options.base_height), (640, 360));
        assert_eq!(options.actual_size(), (1_280, 720));
        assert!(options.dirty);
    }
}

#[derive(Debug, Clone)]
pub struct DisplayOptions {
    base_width: u32,
    base_height: u32,
    pub scale: f32,
    scale_percent: i32,
    pub point_filtering: bool,
    pub mode: DisplayMode,
    pub maximized: bool,
    pub position: Option<(i32, i32)>,
    dirty: bool,
    first_run: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            base_width: DEFAULT_RES_X,
            base_height: DEFAULT_RES_Y,
            scale: 1.0,
            scale_percent: 100,
            point_filtering: false,
            mode: DisplayMode::Fullscreen,
            maximized: false,
            position: None,
            dirty: false,
            first_run: false,
        }
    }
}

impl DisplayOptions {
    pub fn load(paths: Option<&AppPaths>) -> Self {
        let mut options = Self::default();
        let Some(paths) = paths else {
            return options;
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => options.apply_config(&config),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // C4Application saves the freshly defaulted configuration
                // during startup. Keep the missing-file case dirty so the
                // normal shutdown path persists the native fullscreen mode.
                options.dirty = true;
                options.first_run = true;
            }
            Err(err) => tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to load display config"
            ),
        }
        options
    }

    /// Marks a configuration as never having been written to disk.
    #[cfg(test)]
    pub fn mark_first_run(&mut self) {
        self.first_run = true;
    }

    /// Seeds the application scale from the display's own pixel density, but
    /// only when no configuration file exists yet.
    ///
    /// Deliberate divergence: C++ starts every install at `Scale=100`
    /// (src/C4Config.cpp:480), which on a 2x panel means an 800x600 *device
    /// pixel* window and a 14px font. Because `Scale` divides the physical
    /// extent into the logical layout
    /// (`logical_size_for`, crates/clonk-scaling/src/lib.rs:12-17), seeding it
    /// from the monitor keeps the classic 800x600 logical layout and simply
    /// gives it the panel's real pixel density. The scale is rounded to an
    /// integer: a fractional application scale sends every glyph through a
    /// bilinear resample of the native atlas
    /// (`requires_resampling`, crates/clonk-frontend/src/clonk_fonts.rs:102-113).
    ///
    /// Returns whether the scale was changed.
    pub fn apply_first_run_display_scale(&mut self, scale_factor: f64) -> bool {
        if !self.first_run || !scale_factor.is_finite() || scale_factor < 1.0 {
            return false;
        }
        let percent = ((scale_factor.round() as i32).saturating_mul(100))
            .clamp(MIN_GRAPHICS_SCALE_PERCENT, MAX_GRAPHICS_SCALE_PERCENT);
        if percent == self.scale_percent {
            return false;
        }
        self.scale_percent = percent;
        self.scale = percent as f32 / 100.0;
        self.dirty = true;
        true
    }

    /// Window size in output pixels: ResX*Scale truncated like the C++
    /// window setup (C4Application.cpp:183).
    pub fn actual_size(&self) -> (u32, u32) {
        let min = MIN_RESOLUTION as f32;
        let width = ((self.base_width as f32) * self.scale).max(min);
        let height = ((self.base_height as f32) * self.scale).max(min);
        (width as u32, height as u32)
    }

    pub const fn scale_percent(&self) -> i32 {
        self.scale_percent
    }

    /// Commits an accepted application scale while retaining the current
    /// physical window size. C++ recomputes ResolutionX/Y from the physical
    /// extent at the new scale before saving the confirmed value.
    pub fn record_scale_percent(
        &mut self,
        percent: i32,
        physical_width: u32,
        physical_height: u32,
    ) {
        let percent = percent.clamp(1, 10_000);
        let scale = percent as f32 / 100.0;
        if self.scale_percent != percent {
            self.scale_percent = percent;
            self.scale = scale;
            self.dirty = true;
        }
        self.record_actual_size(physical_width, physical_height);
    }

    pub fn checked_loader_actual_size(&self) -> Result<(u32, u32), String> {
        if self.scale_percent <= 0 {
            return Err(format!(
                "classic loader application scale must be positive, got {}%",
                self.scale_percent
            ));
        }
        let percent = u64::try_from(self.scale_percent)
            .map_err(|_| "classic loader application scale is out of range".to_string())?;
        let scaled = |extent: u32, axis: &str| {
            u64::from(extent)
                .checked_mul(percent)
                .map(|value| (value / 100).max(u64::from(MIN_RESOLUTION)))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    format!(
                        "classic loader output {axis} overflows: {extent} * {}%",
                        self.scale_percent
                    )
                })
        };
        let width = scaled(self.base_width, "width").map_err(|_| {
            format!(
                "classic loader output width overflows: {} * {}%",
                self.base_width, self.scale_percent
            )
        })?;
        let height = scaled(self.base_height, "height")?;
        Ok((width, height))
    }

    /// Stores ceil(pixels / scale) like C4Application::SetResolution
    /// (C4Application.cpp:536-538).
    pub fn record_actual_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let scale = self.scale.max(f32::EPSILON);
        let min = MIN_RESOLUTION as f32;
        let max = u32::MAX as f32;
        let new_width = ((width as f32) / scale).ceil().clamp(min, max);
        let new_height = ((height as f32) / scale).ceil().clamp(min, max);
        let new_width = new_width as u32;
        let new_height = new_height as u32;
        if new_width != self.base_width || new_height != self.base_height {
            self.base_width = new_width;
            self.base_height = new_height;
            self.dirty = true;
        }
    }

    pub fn record_position(&mut self, x: i32, y: i32) {
        let new_pos = Some((x, y));
        if self.position != new_pos {
            self.position = new_pos;
            self.dirty = true;
        }
    }

    pub fn record_maximized(&mut self, maximized: bool) {
        if self.maximized != maximized {
            self.maximized = maximized;
            self.dirty = true;
        }
    }

    pub fn record_mode(&mut self, mode: DisplayMode) {
        if self.mode != mode {
            self.mode = mode;
            self.dirty = true;
        }
    }

    pub fn persist_if_dirty(&mut self, paths: &AppPaths) {
        if !self.dirty {
            return;
        }
        let config_path = paths.config_file();
        let mut config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(err) if err.kind() == ErrorKind::NotFound => Config::new(),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to load config for saving display options"
                );
                return;
            }
        };
        self.write_config(&mut config);
        if let Err(err) =
            crate::save_config_preserving_native_general_booleans(&config, &config_path, None, None)
        {
            tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to persist display settings"
            );
            return;
        }
        self.dirty = false;
    }

    /// Writes the C++ engine's key names (C4Config.cpp:440-442); the config
    /// file is shared with the C++ install.
    fn write_config(&self, config: &mut Config) {
        config.set_in(Some("Graphics"), "ResolutionX", self.base_width.to_string());
        config.set_in(
            Some("Graphics"),
            "ResolutionY",
            self.base_height.to_string(),
        );
        config.set_in(Some("Graphics"), "Scale", self.scale_percent.to_string());
        config.set_in(
            Some("Graphics"),
            "PointFiltering",
            if self.point_filtering {
                "true"
            } else {
                "false"
            },
        );
        config.set_in(Some("Graphics"), "DisplayMode", self.mode.to_config_value());
        config.set_in(
            Some("Graphics"),
            "Maximized",
            if self.maximized { "true" } else { "false" },
        );
        if let Some((x, y)) = self.position {
            config.set_in(Some("Graphics"), "PositionX", x.to_string());
            config.set_in(Some("Graphics"), "PositionY", y.to_string());
        }
    }

    fn apply_config(&mut self, config: &Config) {
        if let Some(raw) = config.get_in(Some("Graphics"), "ResolutionX") {
            if let Some(parsed) = parse_native_config_integer(raw) {
                if parsed > 0 {
                    self.base_width = parsed as u32;
                }
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "ResolutionY") {
            if let Some(parsed) = parse_native_config_integer(raw) {
                if parsed > 0 {
                    self.base_height = parsed as u32;
                }
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "Scale") {
            if let Some(percent) = parse_native_config_integer(raw) {
                self.scale_percent = percent;
                self.scale = (percent as f32) / 100.0;
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "PointFiltering") {
            if let Some(parsed) = parse_bool(raw) {
                self.point_filtering = parsed;
            }
        }
        match config.get_in(Some("Graphics"), "DisplayMode") {
            Some(raw) => {
                if let Some(mode) = DisplayMode::from_config(raw) {
                    self.mode = mode;
                }
            }
            // The classic startup save materializes an absent enum default.
            None => self.dirty = true,
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "Maximized") {
            if let Some(parsed) = parse_bool(raw) {
                self.maximized = parsed;
            }
        }
        let pos_x = config
            .get_in(Some("Graphics"), "PositionX")
            .and_then(parse_native_config_integer);
        let pos_y = config
            .get_in(Some("Graphics"), "PositionY")
            .and_then(parse_native_config_integer);
        if let (Some(x), Some(y)) = (pos_x, pos_y) {
            self.position = Some((x, y));
        }
    }
}
