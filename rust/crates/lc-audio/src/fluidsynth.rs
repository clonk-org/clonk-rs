use std::ffi::{c_char, c_int, c_void, CString, OsStr};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use libloading::Library;

use crate::decoder::{AudioDecodeError, DecodedAudio};
use crate::midi::{parse_timeline, MidiCommand, MidiTimeline};

const MAX_RENDER_BLOCK_FRAMES: usize = 4_096;
const RELEASE_POLL_FRAMES: usize = 64;
// `decode_audio` remains an eager public compatibility API (music playback no
// longer uses it). Bound only that collector so hostile sparse MIDI cannot
// force an effectively unbounded Vec allocation; MidiStream itself has no
// playback-duration ceiling.
const MAX_EAGER_DECODE_SECONDS: u64 = 15 * 60;
// This is a synthesizer-liveness bound, not a MIDI duration or PCM allocation
// bound. A broken or hostile SoundFont must not keep a finished stream alive
// forever by reporting an active voice that never releases.
const MAX_VOICE_RELEASE_SECONDS: u64 = 15 * 60;
const SDL_MIXER_FALLBACK_SOUNDFONT: &str = "/usr/share/sounds/sf2/FluidR3_GM.sf2";

pub(crate) struct MidiStream {
    timeline: MidiTimeline,
    synth: FluidSynth,
    state: MidiStreamState,
}

impl MidiStream {
    pub(crate) fn new(data: &[u8], sample_rate: u32) -> Result<Self, AudioDecodeError> {
        let timeline = parse_timeline(data, sample_rate)?;
        validate_timeline(&timeline)?;
        let soundfonts = midi_soundfont_candidates();
        let synth = FluidSynth::new(sample_rate, &soundfonts)?;
        Ok(Self {
            timeline,
            synth,
            state: MidiStreamState::default(),
        })
    }

    pub(crate) fn read_frames(
        &mut self,
        output: &mut [[f32; 2]],
    ) -> Result<usize, AudioDecodeError> {
        read_timeline_frames(&self.timeline, &mut self.synth, &mut self.state, output)
    }

    pub(crate) fn restart(&mut self) -> Result<(), AudioDecodeError> {
        // System reset keeps the loaded SoundFonts while restoring channels and
        // controllers, so a loop never reloads files inside the mixer callback.
        self.synth.reset()?;
        self.state = MidiStreamState::default();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn buffered_frames(&self) -> usize {
        0
    }
}

pub(crate) fn decode_midi(data: &[u8], sample_rate: u32) -> Result<DecodedAudio, AudioDecodeError> {
    let mut stream = MidiStream::new(data, sample_rate)?;
    let max_frames = u64::from(sample_rate)
        .checked_mul(MAX_EAGER_DECODE_SECONDS)
        .and_then(|frames| usize::try_from(frames).ok())
        .ok_or_else(|| midi_error("MIDI eager-decode limit exceeds platform limits"))?;
    let mut frames = Vec::new();
    let mut block = vec![[0.0, 0.0]; MAX_RENDER_BLOCK_FRAMES];
    loop {
        let read = stream.read_frames(&mut block)?;
        if read == 0 {
            break;
        }
        ensure_eager_output_fits(frames.len(), read, max_frames)?;
        frames
            .try_reserve(read)
            .map_err(|_| midi_error("MIDI output is too large"))?;
        frames.extend_from_slice(&block[..read]);
    }
    Ok(DecodedAudio {
        frames,
        sample_rate,
    })
}

fn ensure_eager_output_fits(
    current: usize,
    additional: usize,
    max_frames: usize,
) -> Result<(), AudioDecodeError> {
    current
        .checked_add(additional)
        .filter(|total| *total <= max_frames)
        .map(|_| ())
        .ok_or_else(|| {
            midi_error("MIDI is too long for eager decode; use streaming music playback")
        })
}

trait MidiSynth {
    fn dispatch(&mut self, command: &MidiCommand) -> Result<(), AudioDecodeError>;
    fn finish(&mut self);
    fn active_voice_count(&self) -> usize;
    fn release_poll_frames(&self) -> usize {
        RELEASE_POLL_FRAMES
    }
    fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError>;
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum MidiStreamPhase {
    #[default]
    Body,
    Release,
    Grace,
    Done,
}

#[derive(Default)]
struct MidiStreamState {
    phase: MidiStreamPhase,
    event_index: usize,
    body_frame: usize,
    release_poll_remaining: usize,
    released_frames: usize,
    grace_frames: usize,
}

fn validate_timeline(timeline: &MidiTimeline) -> Result<(), AudioDecodeError> {
    if timeline.sample_rate == 0
        || timeline.body_end_frame > timeline.end_frame
        || timeline
            .events
            .iter()
            .any(|event| event.frame > timeline.body_end_frame)
        || timeline
            .events
            .windows(2)
            .any(|events| events[0].frame > events[1].frame)
    {
        return Err(midi_error("invalid MIDI render timeline"));
    }
    voice_release_frame_limit(timeline.sample_rate)?;
    Ok(())
}

fn voice_release_frame_limit(sample_rate: u32) -> Result<usize, AudioDecodeError> {
    u64::from(sample_rate)
        .checked_mul(MAX_VOICE_RELEASE_SECONDS)
        .and_then(|frames| usize::try_from(frames).ok())
        .ok_or_else(|| midi_error("MIDI voice release safety limit exceeds platform limits"))
}

fn read_timeline_frames<S: MidiSynth>(
    timeline: &MidiTimeline,
    synth: &mut S,
    state: &mut MidiStreamState,
    output: &mut [[f32; 2]],
) -> Result<usize, AudioDecodeError> {
    if output.is_empty() || state.phase == MidiStreamPhase::Done {
        return Ok(0);
    }

    let release_frame_limit = voice_release_frame_limit(timeline.sample_rate)?;
    let grace_frame_limit = timeline.end_frame - timeline.body_end_frame;
    let mut written = 0;
    loop {
        match state.phase {
            MidiStreamPhase::Body => {
                while let Some(event) = timeline.events.get(state.event_index) {
                    if event.frame != state.body_frame {
                        break;
                    }
                    synth.dispatch(&event.command)?;
                    state.event_index += 1;
                }
                if state.body_frame == timeline.body_end_frame {
                    synth.finish();
                    state.phase = MidiStreamPhase::Release;
                    continue;
                }
                if written == output.len() {
                    return Ok(written);
                }

                let next_event_frame = timeline
                    .events
                    .get(state.event_index)
                    .map_or(timeline.body_end_frame, |event| event.frame);
                let count = (next_event_frame - state.body_frame)
                    .min(output.len() - written)
                    .min(MAX_RENDER_BLOCK_FRAMES);
                synth.render(&mut output[written..written + count])?;
                state.body_frame += count;
                written += count;
            }
            MidiStreamPhase::Release => {
                if written == output.len() {
                    return Ok(written);
                }
                if state.release_poll_remaining == 0 {
                    if synth.active_voice_count() == 0 {
                        state.phase = MidiStreamPhase::Grace;
                        continue;
                    }
                    let remaining = release_frame_limit
                        .checked_sub(state.released_frames)
                        .ok_or_else(|| midi_error("MIDI voice release exceeds safety limit"))?;
                    if remaining == 0 {
                        return Err(midi_error("MIDI voice release exceeds safety limit"));
                    }
                    state.release_poll_remaining =
                        synth.release_poll_frames().max(1).min(remaining);
                }

                let count = state
                    .release_poll_remaining
                    .min(output.len() - written)
                    .min(MAX_RENDER_BLOCK_FRAMES);
                synth.render(&mut output[written..written + count])?;
                state.release_poll_remaining -= count;
                state.released_frames += count;
                written += count;
            }
            MidiStreamPhase::Grace => {
                if state.grace_frames == grace_frame_limit {
                    state.phase = MidiStreamPhase::Done;
                    continue;
                }
                if written == output.len() {
                    return Ok(written);
                }
                let count = (grace_frame_limit - state.grace_frames)
                    .min(output.len() - written)
                    .min(MAX_RENDER_BLOCK_FRAMES);
                synth.render(&mut output[written..written + count])?;
                state.grace_frames += count;
                written += count;
            }
            MidiStreamPhase::Done => return Ok(written),
        }
    }
}

#[cfg(test)]
fn render_timeline<S: MidiSynth>(
    timeline: &MidiTimeline,
    synth: &mut S,
) -> Result<Vec<[f32; 2]>, AudioDecodeError> {
    validate_timeline(timeline)?;
    let mut state = MidiStreamState::default();
    let mut frames = Vec::new();
    let mut block = [[0.0, 0.0]; 257];
    loop {
        let read = read_timeline_frames(timeline, synth, &mut state, &mut block)?;
        if read == 0 {
            return Ok(frames);
        }
        frames
            .try_reserve(read)
            .map_err(|_| midi_error("MIDI output is too large"))?;
        frames.extend_from_slice(&block[..read]);
    }
}

struct FluidSynth {
    api: Arc<FluidApi>,
    settings: *mut c_void,
    synth: *mut c_void,
}

// A FluidSynth instance is only accessed through `&mut self`; moving ownership
// to the audio thread does not introduce concurrent access to either C object.
unsafe impl Send for FluidSynth {}

impl FluidSynth {
    fn new(sample_rate: u32, soundfonts: &[PathBuf]) -> Result<Self, AudioDecodeError> {
        let api = FluidApi::load()?;
        Self::with_api(api, sample_rate, soundfonts)
    }

    fn with_api(
        api: Arc<FluidApi>,
        sample_rate: u32,
        soundfonts: &[PathBuf],
    ) -> Result<Self, AudioDecodeError> {
        if sample_rate == 0 {
            return Err(midi_error("MIDI output sample rate is zero"));
        }

        let settings = unsafe { (api.new_fluid_settings)() };
        if settings.is_null() {
            return Err(midi_error("FluidSynth could not create settings"));
        }
        let set_rate = unsafe {
            (api.fluid_settings_setnum)(
                settings,
                c"synth.sample-rate".as_ptr(),
                f64::from(sample_rate),
            )
        };
        if set_rate != 0 {
            unsafe { (api.delete_fluid_settings)(settings) };
            return Err(midi_error("FluidSynth rejected the output sample rate"));
        }
        let mut effective_rate = 0.0;
        let get_rate = unsafe {
            (api.fluid_settings_getnum)(
                settings,
                c"synth.sample-rate".as_ptr(),
                &mut effective_rate,
            )
        };
        if get_rate != 0 || effective_rate != f64::from(sample_rate) {
            unsafe { (api.delete_fluid_settings)(settings) };
            return Err(midi_error(
                "FluidSynth could not use the requested output sample rate",
            ));
        }

        let synth = unsafe { (api.new_fluid_synth)(settings) };
        if synth.is_null() {
            unsafe { (api.delete_fluid_settings)(settings) };
            return Err(midi_error("FluidSynth could not create a synthesizer"));
        }
        unsafe { (api.fluid_synth_set_gain)(synth, 1.0) };

        let instance = Self {
            api,
            settings,
            synth,
        };
        instance.load_soundfonts(soundfonts)?;
        Ok(instance)
    }

    fn load_soundfonts(&self, paths: &[PathBuf]) -> Result<(), AudioDecodeError> {
        let mut loaded = 0_u32;
        let mut last_error = None;
        for path in paths {
            if !path.is_file() {
                continue;
            }
            match has_soundfont_header(path) {
                Ok(true) => {}
                Ok(false) => {
                    last_error = Some(format!("{} is not an SF2/SF3 SoundFont", path.display()));
                    continue;
                }
                Err(error) => {
                    last_error = Some(format!("{}: {error}", path.display()));
                    continue;
                }
            }
            let path_string = match path_to_c_string(path) {
                Ok(path) => path,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let soundfont_id =
                unsafe { (self.api.fluid_synth_sfload)(self.synth, path_string.as_ptr(), 1) };
            if soundfont_id >= 0 {
                loaded += 1;
                tracing::debug!(path = %path.display(), "loaded MIDI SoundFont");
            } else {
                last_error = Some(format!("FluidSynth rejected {}", path.display()));
            }
        }

        if loaded != 0 {
            return Ok(());
        }
        Err(midi_error(last_error.unwrap_or_else(|| {
            "no SoundFont found; set SDL_SOUNDFONTS to an SF2/SF3 path".to_owned()
        })))
    }

    fn reset(&mut self) -> Result<(), AudioDecodeError> {
        let result = unsafe { (self.api.fluid_synth_system_reset)(self.synth) };
        if result != 0 {
            return Err(midi_error("FluidSynth failed to reset MIDI playback"));
        }
        Ok(())
    }
}

impl MidiSynth for FluidSynth {
    fn dispatch(&mut self, command: &MidiCommand) -> Result<(), AudioDecodeError> {
        unsafe {
            match command {
                MidiCommand::NoteOff { channel, key, .. } => {
                    (self.api.fluid_synth_noteoff)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*key),
                    );
                }
                MidiCommand::NoteOn {
                    channel,
                    key,
                    velocity,
                } => {
                    (self.api.fluid_synth_noteon)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*key),
                        c_int::from(*velocity),
                    );
                }
                MidiCommand::Aftertouch {
                    channel,
                    key,
                    pressure,
                } => {
                    (self.api.fluid_synth_key_pressure)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*key),
                        c_int::from(*pressure),
                    );
                }
                MidiCommand::Controller {
                    channel,
                    controller,
                    value,
                } => {
                    (self.api.fluid_synth_cc)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*controller),
                        c_int::from(*value),
                    );
                }
                MidiCommand::ProgramChange { channel, program } => {
                    (self.api.fluid_synth_program_change)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*program),
                    );
                }
                MidiCommand::ChannelAftertouch { channel, pressure } => {
                    (self.api.fluid_synth_channel_pressure)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*pressure),
                    );
                }
                MidiCommand::PitchBend { channel, value } => {
                    (self.api.fluid_synth_pitch_bend)(
                        self.synth,
                        c_int::from(*channel),
                        c_int::from(*value),
                    );
                }
                MidiCommand::SysEx(data) => {
                    let length = c_int::try_from(data.len())
                        .map_err(|_| midi_error("MIDI SysEx message is too large"))?;
                    let mut handled = 0;
                    (self.api.fluid_synth_sysex)(
                        self.synth,
                        data.as_ptr().cast(),
                        length,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut handled,
                        0,
                    );
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) {
        for channel in 0..16 {
            unsafe {
                (self.api.fluid_synth_cc)(self.synth, channel, 64, 0);
                (self.api.fluid_synth_cc)(self.synth, channel, 66, 0);
                (self.api.fluid_synth_cc)(self.synth, channel, 123, 0);
            }
        }
    }

    fn active_voice_count(&self) -> usize {
        let count = unsafe { (self.api.fluid_synth_get_active_voice_count)(self.synth) };
        if count > 0 {
            count as usize
        } else {
            0
        }
    }

    fn release_poll_frames(&self) -> usize {
        let count = unsafe { (self.api.fluid_synth_get_internal_bufsize)(self.synth) };
        usize::try_from(count)
            .ok()
            .filter(|count| *count != 0)
            .unwrap_or(RELEASE_POLL_FRAMES)
    }

    fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError> {
        for block in output.chunks_mut(MAX_RENDER_BLOCK_FRAMES) {
            let length = c_int::try_from(block.len())
                .map_err(|_| midi_error("MIDI render block is too large"))?;
            let samples = block.as_mut_ptr().cast::<f32>();
            let result = unsafe {
                (self.api.fluid_synth_write_float)(
                    self.synth,
                    length,
                    samples.cast(),
                    0,
                    2,
                    samples.cast(),
                    1,
                    2,
                )
            };
            if result != 0 {
                return Err(midi_error("FluidSynth failed to render MIDI audio"));
            }
        }
        Ok(())
    }
}

impl Drop for FluidSynth {
    fn drop(&mut self) {
        unsafe {
            (self.api.delete_fluid_synth)(self.synth);
            (self.api.delete_fluid_settings)(self.settings);
        }
    }
}

struct FluidApi {
    _library: Library,
    new_fluid_settings: unsafe extern "C" fn() -> *mut c_void,
    delete_fluid_settings: unsafe extern "C" fn(*mut c_void),
    fluid_settings_setnum: unsafe extern "C" fn(*mut c_void, *const c_char, f64) -> c_int,
    fluid_settings_getnum: unsafe extern "C" fn(*mut c_void, *const c_char, *mut f64) -> c_int,
    new_fluid_synth: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    delete_fluid_synth: unsafe extern "C" fn(*mut c_void),
    fluid_synth_set_gain: unsafe extern "C" fn(*mut c_void, f32),
    fluid_synth_sfload: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_int,
    fluid_synth_write_float: unsafe extern "C" fn(
        *mut c_void,
        c_int,
        *mut c_void,
        c_int,
        c_int,
        *mut c_void,
        c_int,
        c_int,
    ) -> c_int,
    fluid_synth_noteon: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int,
    fluid_synth_noteoff: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    fluid_synth_cc: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int,
    fluid_synth_program_change: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    fluid_synth_channel_pressure: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    fluid_synth_key_pressure: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int,
    fluid_synth_pitch_bend: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    fluid_synth_system_reset: unsafe extern "C" fn(*mut c_void) -> c_int,
    fluid_synth_get_active_voice_count: unsafe extern "C" fn(*mut c_void) -> c_int,
    fluid_synth_get_internal_bufsize: unsafe extern "C" fn(*mut c_void) -> c_int,
    fluid_synth_sysex: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        c_int,
        *mut c_char,
        *mut c_int,
        *mut c_int,
        c_int,
    ) -> c_int,
}

impl FluidApi {
    fn load() -> Result<Arc<Self>, AudioDecodeError> {
        let mut last_error = None;
        for path in fluid_library_candidates() {
            let library = match unsafe { Library::new(&path) } {
                Ok(library) => library,
                Err(error) => {
                    last_error = Some(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            match unsafe { Self::from_library(library) } {
                Ok(api) => return Ok(Arc::new(api)),
                Err(error) => last_error = Some(format!("{}: {error}", path.display())),
            }
        }
        Err(midi_error(last_error.unwrap_or_else(|| {
            "FluidSynth library not found; set LC_FLUIDSYNTH_LIBRARY".to_owned()
        })))
    }

    unsafe fn from_library(library: Library) -> Result<Self, String> {
        let fluid_version: unsafe extern "C" fn(*mut c_int, *mut c_int, *mut c_int) =
            unsafe { load_symbol(&library, b"fluid_version\0")? };
        let mut major = 0;
        let mut minor = 0;
        let mut micro = 0;
        unsafe { fluid_version(&mut major, &mut minor, &mut micro) };
        if major != 2 {
            return Err(format!(
                "unsupported FluidSynth ABI version {major}.{minor}.{micro}"
            ));
        }
        if (minor, micro) < (5, 6) {
            static WARN_OLD_FLUIDSYNTH: Once = Once::new();
            WARN_OLD_FLUIDSYNTH.call_once(|| {
                tracing::warn!(
                    version = %format!("{major}.{minor}.{micro}"),
                    "FluidSynth is older than 2.5.6; use only trusted SoundFonts and upgrade for current security fixes"
                );
            });
        }

        Ok(Self {
            new_fluid_settings: unsafe { load_symbol(&library, b"new_fluid_settings\0")? },
            delete_fluid_settings: unsafe { load_symbol(&library, b"delete_fluid_settings\0")? },
            fluid_settings_setnum: unsafe { load_symbol(&library, b"fluid_settings_setnum\0")? },
            fluid_settings_getnum: unsafe { load_symbol(&library, b"fluid_settings_getnum\0")? },
            new_fluid_synth: unsafe { load_symbol(&library, b"new_fluid_synth\0")? },
            delete_fluid_synth: unsafe { load_symbol(&library, b"delete_fluid_synth\0")? },
            fluid_synth_set_gain: unsafe { load_symbol(&library, b"fluid_synth_set_gain\0")? },
            fluid_synth_sfload: unsafe { load_symbol(&library, b"fluid_synth_sfload\0")? },
            fluid_synth_write_float: unsafe {
                load_symbol(&library, b"fluid_synth_write_float\0")?
            },
            fluid_synth_noteon: unsafe { load_symbol(&library, b"fluid_synth_noteon\0")? },
            fluid_synth_noteoff: unsafe { load_symbol(&library, b"fluid_synth_noteoff\0")? },
            fluid_synth_cc: unsafe { load_symbol(&library, b"fluid_synth_cc\0")? },
            fluid_synth_program_change: unsafe {
                load_symbol(&library, b"fluid_synth_program_change\0")?
            },
            fluid_synth_channel_pressure: unsafe {
                load_symbol(&library, b"fluid_synth_channel_pressure\0")?
            },
            fluid_synth_key_pressure: unsafe {
                load_symbol(&library, b"fluid_synth_key_pressure\0")?
            },
            fluid_synth_pitch_bend: unsafe { load_symbol(&library, b"fluid_synth_pitch_bend\0")? },
            fluid_synth_system_reset: unsafe {
                load_symbol(&library, b"fluid_synth_system_reset\0")?
            },
            fluid_synth_get_active_voice_count: unsafe {
                load_symbol(&library, b"fluid_synth_get_active_voice_count\0")?
            },
            fluid_synth_get_internal_bufsize: unsafe {
                load_symbol(&library, b"fluid_synth_get_internal_bufsize\0")?
            },
            fluid_synth_sysex: unsafe { load_symbol(&library, b"fluid_synth_sysex\0")? },
            _library: library,
        })
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| error.to_string())
}

fn fluid_library_candidates() -> Vec<PathBuf> {
    if let Some(configured) = std::env::var_os("LC_FLUIDSYNTH_LIBRARY") {
        return vec![PathBuf::from(configured)];
    }

    let mut candidates = Vec::new();
    #[cfg(target_os = "macos")]
    candidates.extend(
        [
            "/opt/homebrew/opt/fluid-synth/lib/libfluidsynth.3.dylib",
            "/opt/homebrew/lib/libfluidsynth.3.dylib",
            "/usr/local/opt/fluid-synth/lib/libfluidsynth.3.dylib",
            "/usr/local/lib/libfluidsynth.3.dylib",
            "/opt/local/lib/libfluidsynth.3.dylib",
            "libfluidsynth.3.dylib",
            "libfluidsynth.2.dylib",
            "libfluidsynth.dylib",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    candidates.extend(
        [
            "libfluidsynth.so.3",
            "libfluidsynth.so.2",
            "libfluidsynth.so",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    #[cfg(windows)]
    if let Some(directory) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        candidates.extend(
            [
                "libfluidsynth-3.dll",
                "libfluidsynth-2.dll",
                "libfluidsynth.dll",
                "fluidsynth.dll",
            ]
            .into_iter()
            .map(|name| directory.join(name)),
        );
    }
    candidates
}

fn midi_soundfont_candidates() -> Vec<PathBuf> {
    let configured = std::env::var_os("SDL_SOUNDFONTS");
    resolve_soundfont_candidates(configured.as_deref(), |path| File::open(path).is_ok())
}

fn resolve_soundfont_candidates(
    configured: Option<&OsStr>,
    fallback_is_readable: impl FnOnce(&Path) -> bool,
) -> Vec<PathBuf> {
    configured
        .filter(|paths| !paths.is_empty())
        .map(parse_soundfont_list)
        .unwrap_or_else(|| {
            let fallback = PathBuf::from(SDL_MIXER_FALLBACK_SOUNDFONT);
            fallback_is_readable(&fallback)
                .then_some(fallback)
                .into_iter()
                .collect()
        })
}

fn parse_soundfont_list(configured: &OsStr) -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        configured
            .as_bytes()
            .split(|byte| *byte == b';')
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
            .collect()
    }
    #[cfg(not(unix))]
    {
        configured
            .to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect()
    }
}

fn has_soundfont_header(path: &Path) -> Result<bool, std::io::Error> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header)?;
    Ok(&header[0..4] == b"RIFF" && &header[8..12] == b"sfbk")
}

fn path_to_c_string(path: &Path) -> Result<CString, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("SoundFont path contains NUL: {}", path.display()))
    }
    #[cfg(not(unix))]
    {
        path.to_str()
            .ok_or_else(|| format!("SoundFont path is not valid Unicode: {}", path.display()))
            .and_then(|path| {
                CString::new(path).map_err(|_| {
                    format!("SoundFont path contains NUL: {}", Path::new(path).display())
                })
            })
    }
}

fn midi_error(message: impl Into<String>) -> AudioDecodeError {
    AudioDecodeError::MidiDecoderError(message.into())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::midi::{MidiCommand, MidiTimeline, TimedMidiEvent};

    #[derive(Default)]
    struct FakeSynth {
        active: bool,
        commands: Vec<MidiCommand>,
    }

    impl MidiSynth for FakeSynth {
        fn dispatch(&mut self, command: &MidiCommand) -> Result<(), AudioDecodeError> {
            self.active = matches!(command, MidiCommand::NoteOn { .. });
            self.commands.push(command.clone());
            Ok(())
        }

        fn finish(&mut self) {
            self.active = false;
        }

        fn active_voice_count(&self) -> usize {
            usize::from(self.active)
        }

        fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError> {
            output.fill(if self.active {
                [0.25, 0.25]
            } else {
                [0.0, 0.0]
            });
            Ok(())
        }
    }

    #[derive(Default)]
    struct ReleasingSynth {
        active: bool,
        finishing: bool,
    }

    #[derive(Default)]
    struct NeverReleasingSynth {
        active: bool,
        rendered: usize,
    }

    #[derive(Default)]
    struct CountingSynth {
        rendered: usize,
        largest_block: usize,
        finish_count: usize,
    }

    impl MidiSynth for ReleasingSynth {
        fn dispatch(&mut self, command: &MidiCommand) -> Result<(), AudioDecodeError> {
            if matches!(command, MidiCommand::NoteOn { .. }) {
                self.active = true;
            }
            Ok(())
        }

        fn finish(&mut self) {
            self.finishing = true;
        }

        fn active_voice_count(&self) -> usize {
            usize::from(self.active)
        }

        fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError> {
            output.fill(if self.active {
                [0.25, 0.25]
            } else {
                [0.0, 0.0]
            });
            if self.finishing {
                self.active = false;
            }
            Ok(())
        }
    }

    impl MidiSynth for NeverReleasingSynth {
        fn dispatch(&mut self, command: &MidiCommand) -> Result<(), AudioDecodeError> {
            self.active = matches!(command, MidiCommand::NoteOn { .. });
            Ok(())
        }

        fn finish(&mut self) {}

        fn active_voice_count(&self) -> usize {
            usize::from(self.active)
        }

        fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError> {
            self.rendered += output.len();
            Ok(())
        }
    }

    impl MidiSynth for CountingSynth {
        fn dispatch(&mut self, _command: &MidiCommand) -> Result<(), AudioDecodeError> {
            Ok(())
        }

        fn finish(&mut self) {
            self.finish_count += 1;
        }

        fn active_voice_count(&self) -> usize {
            0
        }

        fn render(&mut self, output: &mut [[f32; 2]]) -> Result<(), AudioDecodeError> {
            self.rendered += output.len();
            self.largest_block = self.largest_block.max(output.len());
            output.fill([0.0, 0.0]);
            Ok(())
        }
    }

    #[test]
    fn configured_soundfonts_use_sdl_mixer_delimiter() {
        // SDL_mixer searches its path list in reverse priority by loading each
        // SoundFont in semicolon-delimited order; C4AudioSystemSdl.cpp:280-282
        // uses that backend.
        assert_eq!(
            parse_soundfont_list(OsStr::new("base.sf2;override.sf2")),
            vec![PathBuf::from("base.sf2"), PathBuf::from("override.sf2")]
        );
        #[cfg(unix)]
        assert_eq!(
            parse_soundfont_list(OsStr::new("base.sf2:override.sf2")),
            vec![PathBuf::from("base.sf2:override.sf2")]
        );
    }

    #[test]
    fn configured_soundfonts_override_implicit_fallback() {
        let configured = OsStr::new("trusted-base.sf2;trusted-override.sf2");

        assert_eq!(
            resolve_soundfont_candidates(Some(configured), |_| {
                panic!("explicit SoundFonts must bypass implicit discovery")
            }),
            vec![
                PathBuf::from("trusted-base.sf2"),
                PathBuf::from("trusted-override.sf2")
            ]
        );
    }

    #[test]
    fn implicit_soundfonts_match_sdl_mixer_fallback() {
        // C4AudioSystemSdl.cpp:280-282 delegates MIDI loading to SDL_mixer 2.8.1,
        // whose sole implicit SoundFont is FluidR3_GM at this exact path.
        let fallback = PathBuf::from("/usr/share/sounds/sf2/FluidR3_GM.sf2");

        assert_eq!(
            resolve_soundfont_candidates(None, |path| path == fallback),
            vec![fallback]
        );
        assert!(resolve_soundfont_candidates(None, |_| false).is_empty());
    }

    #[test]
    fn renderer_dispatches_at_exact_frames_then_releases_at_eot() {
        let timeline = MidiTimeline {
            events: vec![
                TimedMidiEvent {
                    frame: 2,
                    command: MidiCommand::NoteOn {
                        channel: 0,
                        key: 60,
                        velocity: 100,
                    },
                },
                TimedMidiEvent {
                    frame: 5,
                    command: MidiCommand::NoteOff {
                        channel: 0,
                        key: 60,
                        velocity: 64,
                    },
                },
            ],
            body_end_frame: 7,
            end_frame: 9,
            sample_rate: 1,
        };
        let mut synth = FakeSynth::default();

        let frames = render_timeline(&timeline, &mut synth).expect("fake synthesis");

        assert_eq!(&frames[..2], &[[0.0, 0.0]; 2]);
        assert_eq!(&frames[2..5], &[[0.25, 0.25]; 3]);
        assert_eq!(&frames[5..], &[[0.0, 0.0]; 4]);
        assert_eq!(
            synth.commands,
            timeline
                .events
                .iter()
                .map(|event| event.command.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn renderer_waits_for_voice_release_before_the_grace_tail() {
        // C4AudioSystemSdl.cpp:280-282 delegates MIDI playback to SDL_mixer's
        // FluidSynth backend, which drains active voices before its grace tail.
        let timeline = MidiTimeline {
            events: vec![TimedMidiEvent {
                frame: 0,
                command: MidiCommand::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                },
            }],
            body_end_frame: 7,
            end_frame: 9,
            sample_rate: 1,
        };
        let mut synth = ReleasingSynth::default();

        let frames = render_timeline(&timeline, &mut synth).expect("fake release synthesis");

        assert_eq!(frames.len(), 7 + 64 + 2);
    }

    #[test]
    fn long_music_streams_with_bounded_memory_and_no_prerender_limit() {
        let sample_rate = 1_000;
        let long_midi = [
            b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 0, 1, b'M', b'T', b'r', b'k', 0, 0, 0,
            5, 0x8e, 0x09, 0xff, 0x2f, 0,
        ];
        let timeline = parse_timeline(&long_midi, sample_rate).expect("long sparse MIDI");
        assert!(timeline.body_end_frame > 15 * 60 * sample_rate as usize);
        validate_timeline(&timeline).expect("long timeline");
        let mut synth = CountingSynth::default();
        let mut state = MidiStreamState::default();
        let mut output = [[0.0, 0.0]; 31];

        let read = read_timeline_frames(&timeline, &mut synth, &mut state, &mut output)
            .expect("bounded streaming prefix");

        assert_eq!(read, output.len());
        assert_eq!(synth.rendered, output.len());
        assert!(synth.largest_block <= output.len());
        assert!(synth.largest_block <= MAX_RENDER_BLOCK_FRAMES);
        assert_eq!(synth.finish_count, 0);
        assert_eq!(state.body_frame, output.len());
    }

    #[test]
    fn eager_decode_keeps_an_independent_allocation_ceiling() {
        assert!(ensure_eager_output_fits(899, 1, 900).is_ok());
        assert!(ensure_eager_output_fits(900, 1, 900).is_err());
        assert!(ensure_eager_output_fits(usize::MAX, 1, usize::MAX).is_err());
    }

    #[test]
    fn renderer_state_can_restart_after_eof() {
        let timeline = MidiTimeline {
            events: Vec::new(),
            body_end_frame: 1,
            end_frame: 3,
            sample_rate: 1,
        };
        let mut synth = CountingSynth::default();
        let mut state = MidiStreamState::default();
        let mut output = [[0.0, 0.0]; 8];

        assert_eq!(
            read_timeline_frames(&timeline, &mut synth, &mut state, &mut output).unwrap(),
            3
        );
        assert_eq!(
            read_timeline_frames(&timeline, &mut synth, &mut state, &mut output).unwrap(),
            0
        );
        state = MidiStreamState::default();
        synth = CountingSynth::default();

        assert_eq!(
            read_timeline_frames(&timeline, &mut synth, &mut state, &mut output).unwrap(),
            3
        );
        assert_eq!(synth.finish_count, 1);
    }

    #[test]
    fn renderer_rejects_a_voice_that_never_releases() {
        // This independent liveness ceiling protects against a pathological
        // SoundFont; it does not limit the MIDI body's playback duration.
        let timeline = MidiTimeline {
            events: vec![TimedMidiEvent {
                frame: 0,
                command: MidiCommand::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                },
            }],
            body_end_frame: 0,
            end_frame: 2,
            sample_rate: 1,
        };
        let mut synth = NeverReleasingSynth::default();

        let error = render_timeline(&timeline, &mut synth).expect_err("release must be bounded");

        assert!(error.to_string().contains("voice release exceeds"));
        assert_eq!(synth.rendered, MAX_VOICE_RELEASE_SECONDS as usize);
    }

    #[test]
    fn native_fluidsynth_renders_a_valid_one_note_midi_when_available() {
        let soundfont = midi_soundfont_candidates()
            .into_iter()
            .find(|path| matches!(has_soundfont_header(path), Ok(true)));
        let Some(soundfont) = soundfont else {
            return;
        };
        let Ok(api) = FluidApi::load() else {
            return;
        };
        let midi = [
            b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 0, 96, b'M', b'T', b'r', b'k', 0, 0, 0,
            15, 0, 0xC0, 0, 0, 0x90, 60, 100, 96, 0x80, 60, 64, 0, 0xFF, 0x2F, 0,
        ];
        let timeline = parse_timeline(&midi, 8_000).expect("valid one-note MIDI");
        let mut synth =
            FluidSynth::with_api(api, 8_000, &[soundfont]).expect("trusted SoundFont loads");

        let frames = render_timeline(&timeline, &mut synth).expect("native MIDI synthesis");

        assert!(frames.len() >= 20_000);
        assert!(
            frames.len()
                <= timeline.end_frame + 8_000 * usize::try_from(MAX_VOICE_RELEASE_SECONDS).unwrap()
        );
        assert!(frames.iter().any(|frame| frame != &[0.0, 0.0]));
    }
}
