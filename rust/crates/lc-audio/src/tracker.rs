use std::ffi::{c_char, c_int, c_long, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::Library;

use crate::decoder::{AudioDecodeError, DecodedAudio};

const XMP_END: c_int = 1;
const XMP_ERROR_FORMAT: c_int = 3;
const XMP_FORMAT_DEFAULT: c_int = 0;
const XMP_MIN_SAMPLE_RATE: u32 = 4_000;
// libxmp 4.5/4.6, including the Ubuntu runtime package, accepts at most 49,170
// Hz. Newer releases widened that range, but rendering at C4's requested
// 44.1 kHz keeps one decoder path compatible and lets AudioMixer perform the
// same output-device conversion it already applies to every decoded clip.
const XMP_COMPAT_MAX_SAMPLE_RATE: u32 = 49_170;
const C4_TRACKER_SAMPLE_RATE: u32 = 44_100;
// C4AudioSystemSdl opens SDL_mixer with 1,024 stereo S16 samples. Its libxmp
// interface requests exactly that block size and retains the terminal block's
// zero padding before the following call reports XMP_END.
const RENDER_BLOCK_FRAMES: usize = 1_024;
// `decode_audio` remains an eager public compatibility API (music playback no
// longer uses it). Bound only that collector so a hostile module cannot force
// an effectively unbounded Vec allocation; TrackerStream itself has no
// playback-duration ceiling.
const MAX_EAGER_DECODE_SECONDS: u64 = 15 * 60;

type XmpContext = *mut c_char;

pub(crate) struct TrackerStream {
    context: TrackerContext,
    sample_rate: u32,
    interleaved: [i16; RENDER_BLOCK_FRAMES * 2],
    block_position: usize,
}

impl TrackerStream {
    pub(crate) fn new(data: &[u8], requested_sample_rate: u32) -> Result<Self, AudioDecodeError> {
        let sample_rate = compatible_sample_rate(requested_sample_rate);
        let api = LibXmpApi::load()?;
        let mut context = TrackerContext::new(api)?;
        context.load(data)?;
        context.start(sample_rate)?;
        Ok(Self {
            context,
            sample_rate,
            interleaved: [0; RENDER_BLOCK_FRAMES * 2],
            block_position: RENDER_BLOCK_FRAMES,
        })
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        if self.block_position == RENDER_BLOCK_FRAMES && !self.render_block()? {
            return Ok(None);
        }

        let sample = self.block_position * 2;
        self.block_position += 1;
        Ok(Some([
            f32::from(self.interleaved[sample]) / f32::from(i16::MAX),
            f32::from(self.interleaved[sample + 1]) / f32::from(i16::MAX),
        ]))
    }

    pub(crate) fn restart(&mut self) -> Result<(), AudioDecodeError> {
        unsafe { (self.context.api.restart_module)(self.context.raw) };
        self.block_position = RENDER_BLOCK_FRAMES;
        Ok(())
    }

    fn render_block(&mut self) -> Result<bool, AudioDecodeError> {
        let result = unsafe {
            (self.context.api.play_buffer)(
                self.context.raw,
                self.interleaved.as_mut_ptr().cast(),
                c_int::try_from(std::mem::size_of_val(&self.interleaved))
                    .map_err(|_| tracker_error("tracker render block is too large"))?,
                1,
            )
        };
        if result == -XMP_END {
            return Ok(false);
        }
        if result != 0 {
            return Err(xmp_error("render tracker audio", result));
        }
        self.block_position = 0;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn buffered_frames(&self) -> usize {
        RENDER_BLOCK_FRAMES
    }

    #[cfg(test)]
    pub(crate) fn peak_buffered_frames(&self) -> usize {
        RENDER_BLOCK_FRAMES
    }
}

fn compatible_sample_rate(requested_sample_rate: u32) -> u32 {
    if (XMP_MIN_SAMPLE_RATE..=XMP_COMPAT_MAX_SAMPLE_RATE).contains(&requested_sample_rate) {
        requested_sample_rate
    } else {
        C4_TRACKER_SAMPLE_RATE
    }
}

pub(crate) fn decode_tracker(
    data: &[u8],
    output_sample_rate: u32,
) -> Result<DecodedAudio, AudioDecodeError> {
    let sample_rate = compatible_sample_rate(output_sample_rate);

    let api = LibXmpApi::load()?;
    let mut context = TrackerContext::new(api)?;
    context.load(data)?;
    context.start(sample_rate)?;

    let max_frames = u64::from(sample_rate)
        .checked_mul(MAX_EAGER_DECODE_SECONDS)
        .and_then(|frames| usize::try_from(frames).ok())
        .ok_or_else(|| tracker_error("tracker eager-decode limit exceeds platform limits"))?;
    let mut frames = Vec::new();
    let mut interleaved = vec![0_i16; RENDER_BLOCK_FRAMES * 2];
    loop {
        let result = unsafe {
            (context.api.play_buffer)(
                context.raw,
                interleaved.as_mut_ptr().cast(),
                c_int::try_from(std::mem::size_of_val(interleaved.as_slice()))
                    .map_err(|_| tracker_error("tracker render block is too large"))?,
                1,
            )
        };
        if result == -XMP_END {
            break;
        }
        if result != 0 {
            return Err(xmp_error("render tracker audio", result));
        }
        // Permit the decoder's next call at the ceiling so a module whose
        // terminal padded block straddles the limit can report XMP_END. A
        // successful additional block proves that the module exceeds it.
        if frames.len() >= max_frames {
            return Err(tracker_error(
                "tracker is too long for eager decode; use streaming music playback",
            ));
        }
        frames
            .try_reserve(RENDER_BLOCK_FRAMES)
            .map_err(|_| tracker_error("tracker output is too large"))?;
        frames.extend(interleaved.chunks_exact(2).map(|sample| {
            [
                f32::from(sample[0]) / f32::from(i16::MAX),
                f32::from(sample[1]) / f32::from(i16::MAX),
            ]
        }));
    }

    Ok(DecodedAudio {
        frames,
        sample_rate,
    })
}

/// SDL_mixer's MUS_MOD path identifies module bytes in the decoder, not from
/// the selected filename. Keep common family signatures on the fast path and
/// let libxmp probe legacy signature-less variants below.
pub(crate) fn looks_like_tracker(data: &[u8]) -> bool {
    data.starts_with(b"IMPM")
        || data.starts_with(b"Extended Module: ")
        || data.get(44..48) == Some(b"SCRM".as_slice())
        || data.get(1080..1084).is_some_and(is_mod_signature)
}

pub(crate) fn probe_tracker(data: &[u8]) -> Result<bool, AudioDecodeError> {
    let Ok(api) = LibXmpApi::load() else {
        return Ok(false);
    };
    let mut context = TrackerContext::new(api)?;
    let size = c_long::try_from(data.len())
        .map_err(|_| tracker_error("tracker input is too large to probe"))?;
    let result =
        unsafe { (context.api.load_module_from_memory)(context.raw, data.as_ptr().cast(), size) };
    if result == 0 {
        context.module_loaded = true;
        return Ok(true);
    }
    if result == -XMP_ERROR_FORMAT {
        return Ok(false);
    }
    Err(xmp_error("probe tracker data", result))
}

fn is_mod_signature(signature: &[u8]) -> bool {
    matches!(
        signature,
        b"M.K." | b"M!K!" | b"M&K!" | b"N.T." | b"FLT4" | b"FLT8" | b"CD81" | b"OKTA" | b"OCTA"
    ) || (signature[0].is_ascii_digit() && &signature[1..] == b"CHN")
        || (signature[0].is_ascii_digit()
            && signature[1].is_ascii_digit()
            && matches!(&signature[2..], b"CH" | b"CN"))
}

struct TrackerContext {
    api: Arc<LibXmpApi>,
    raw: XmpContext,
    module_loaded: bool,
    player_started: bool,
}

// SAFETY: TrackerContext uniquely owns its xmp_context. Every operation on it
// requires `&mut self`, it is never shared with another thread, and libxmp
// permits independently created contexts to be used by different threads.
unsafe impl Send for TrackerContext {}

impl TrackerContext {
    fn new(api: Arc<LibXmpApi>) -> Result<Self, AudioDecodeError> {
        let raw = unsafe { (api.create_context)() };
        if raw.is_null() {
            return Err(tracker_error("libxmp could not create a decoder context"));
        }
        Ok(Self {
            api,
            raw,
            module_loaded: false,
            player_started: false,
        })
    }

    fn load(&mut self, data: &[u8]) -> Result<(), AudioDecodeError> {
        let size = c_long::try_from(data.len())
            .map_err(|_| tracker_error("tracker input is too large"))?;
        let result =
            unsafe { (self.api.load_module_from_memory)(self.raw, data.as_ptr().cast(), size) };
        if result != 0 {
            return Err(xmp_error("load tracker data", result));
        }
        self.module_loaded = true;
        Ok(())
    }

    fn start(&mut self, sample_rate: u32) -> Result<(), AudioDecodeError> {
        let result = unsafe {
            (self.api.start_player)(
                self.raw,
                c_int::try_from(sample_rate)
                    .map_err(|_| tracker_error("tracker output sample rate is too large"))?,
                XMP_FORMAT_DEFAULT,
            )
        };
        if result != 0 {
            return Err(xmp_error("start tracker decoder", result));
        }
        self.player_started = true;
        Ok(())
    }
}

impl Drop for TrackerContext {
    fn drop(&mut self) {
        unsafe {
            if self.player_started {
                (self.api.end_player)(self.raw);
            }
            if self.module_loaded {
                (self.api.release_module)(self.raw);
            }
            (self.api.free_context)(self.raw);
        }
    }
}

struct LibXmpApi {
    _library: Library,
    create_context: unsafe extern "C" fn() -> XmpContext,
    free_context: unsafe extern "C" fn(XmpContext),
    load_module_from_memory: unsafe extern "C" fn(XmpContext, *const c_void, c_long) -> c_int,
    release_module: unsafe extern "C" fn(XmpContext),
    start_player: unsafe extern "C" fn(XmpContext, c_int, c_int) -> c_int,
    play_buffer: unsafe extern "C" fn(XmpContext, *mut c_void, c_int, c_int) -> c_int,
    restart_module: unsafe extern "C" fn(XmpContext),
    end_player: unsafe extern "C" fn(XmpContext),
}

impl LibXmpApi {
    fn load() -> Result<Arc<Self>, AudioDecodeError> {
        let mut last_error = None;
        for path in library_candidates() {
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
        let detail =
            last_error.unwrap_or_else(|| "no library candidates were available".to_owned());
        Err(tracker_error(format!(
            "libxmp library not found; set LC_LIBXMP_LIBRARY to its path; last error: {detail}"
        )))
    }

    unsafe fn from_library(library: Library) -> Result<Self, String> {
        Ok(Self {
            create_context: unsafe { load_symbol(&library, b"xmp_create_context\0")? },
            free_context: unsafe { load_symbol(&library, b"xmp_free_context\0")? },
            load_module_from_memory: unsafe {
                load_symbol(&library, b"xmp_load_module_from_memory\0")?
            },
            release_module: unsafe { load_symbol(&library, b"xmp_release_module\0")? },
            start_player: unsafe { load_symbol(&library, b"xmp_start_player\0")? },
            play_buffer: unsafe { load_symbol(&library, b"xmp_play_buffer\0")? },
            restart_module: unsafe { load_symbol(&library, b"xmp_restart_module\0")? },
            end_player: unsafe { load_symbol(&library, b"xmp_end_player\0")? },
            _library: library,
        })
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| error.to_string())
}

fn library_candidates() -> Vec<PathBuf> {
    if let Some(configured) = std::env::var_os("LC_LIBXMP_LIBRARY") {
        return vec![PathBuf::from(configured)];
    }

    let mut candidates = Vec::new();
    if let Some(executable_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        candidates.extend(
            platform_library_names()
                .iter()
                .map(|name| executable_dir.join(name)),
        );
        #[cfg(target_os = "macos")]
        if let Some(contents_dir) = executable_dir.parent() {
            candidates.extend(
                platform_library_names()
                    .iter()
                    .map(|name| contents_dir.join("Frameworks").join(name)),
            );
        }
    }

    #[cfg(target_os = "macos")]
    candidates.extend(
        [
            "/opt/homebrew/opt/libxmp/lib/libxmp.4.dylib",
            "/opt/homebrew/lib/libxmp.4.dylib",
            "/usr/local/opt/libxmp/lib/libxmp.4.dylib",
            "/usr/local/lib/libxmp.4.dylib",
            "/opt/local/lib/libxmp.4.dylib",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    // Bare DLL names would re-enable the unsafe legacy Windows search order,
    // including the process working directory. Windows candidates above are
    // therefore restricted to the executable directory unless explicitly
    // overridden with LC_LIBXMP_LIBRARY.
    #[cfg(not(windows))]
    candidates.extend(platform_library_names().iter().map(PathBuf::from));
    candidates
}

#[cfg(target_os = "macos")]
fn platform_library_names() -> &'static [&'static str] {
    &["libxmp.4.dylib", "libxmp.dylib"]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_library_names() -> &'static [&'static str] {
    &["libxmp.so.4", "libxmp.so"]
}

#[cfg(windows)]
fn platform_library_names() -> &'static [&'static str] {
    &["libxmp-4.dll", "libxmp.dll", "xmp.dll"]
}

fn xmp_error(operation: &str, result: c_int) -> AudioDecodeError {
    let description = match result.checked_abs() {
        Some(2) => "internal decoder error",
        Some(3) => "unsupported or malformed module format",
        Some(4) => "module load error",
        Some(5) => "module depacking error",
        Some(6) => "decoder system error",
        Some(7) => "invalid decoder parameter",
        Some(8) => "invalid decoder state",
        _ => "unknown decoder error",
    };
    tracker_error(format!(
        "libxmp could not {operation}: {description} ({result})"
    ))
}

fn tracker_error(message: impl Into<String>) -> AudioDecodeError {
    AudioDecodeError::TrackerDecoderError(message.into())
}
