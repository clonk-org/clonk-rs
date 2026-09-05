use super::*;
use crate::SynchronousSoundHostHandle;

pub(crate) fn music(args: &[Value]) -> Result<Value, RuntimeError> {
    let song = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) => Some(name.as_ref().to_owned()),
        Value::Nil | Value::Int(0) => None,
        other => {
            return Err(RuntimeError::new(format!(
                "Music: expected string, nil, or 0 for song name, got {}",
                other.type_name()
            )));
        }
    };
    let looped = args.get(1).is_some_and(Value::as_bool);

    with_host_context_mut((), |context| match song {
        Some(name) => context.audio_mut().play_music(name, looped),
        None => context.audio_mut().stop_music(),
    });
    Ok(Value::Nil)
}

pub(crate) fn music_level(args: &[Value]) -> Result<Value, RuntimeError> {
    let level = value_to_i32(args.first().unwrap_or(&Value::Nil), "MusicLevel", "level")?;
    let level = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.audio_mut().set_music_level(level))
            .unwrap_or(level.clamp(0, 100) as u8)
    });
    Ok(Value::Int(i32::from(level)))
}

/// FnSetPlayList (C4Script.cpp:2349-2356): replace the active music filter,
/// optionally restart automatic playback, and only expose the local match
/// count outside synchronized control modes. The playlist mutation happens
/// before the sync-safe return gate, just like C++.
pub(crate) fn set_play_list(args: &[Value]) -> Result<Value, RuntimeError> {
    let playlist = parse_native_c4_string_argument(args.first(), "SetPlayList", "playlist")?
        .unwrap_or_default();
    let restart = args.get(1).is_some_and(Value::as_bool);

    with_host_context_mut(Ok(Value::Nil), |context| {
        let count = context.audio_mut().set_music_playlist(playlist, restart);
        if context.world.control_sync_mode {
            Ok(Value::Nil)
        } else {
            Ok(Value::Int(count))
        }
    })
}

pub(crate) fn sound(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnSound never inspects the name: FnStringPar turns a null C4String into
    // "" and StartSoundEffect/StopSoundEffect simply resolve nothing, leaving
    // the unconditional network-safe `return true` (C4Script.cpp:2297-2327).
    // An ActMap action without `Sound=` reaches script as exactly that empty
    // string, so it must not abort the caller.
    let name = parse_native_c4_string_argument(args.first(), "Sound", "name")?.unwrap_or_default();

    let mut index = 1;
    let global = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "Sound", "global")?;
        index += 1;
        flag
    } else {
        false
    };

    let object_value = if let Some(arg) = args.get(index) {
        index += 1;
        Some(arg)
    } else {
        None
    };

    let level = if let Some(arg) = args.get(index) {
        index += 1;
        value_to_i32(arg, "Sound", "level")?
    } else {
        0
    };

    let at_player = if let Some(arg) = args.get(index) {
        let player = value_to_i32(arg, "Sound", "at_player")?;
        index += 1;
        player
    } else {
        0
    };

    let loop_flag = if let Some(arg) = args.get(index) {
        index += 1;
        value_to_i32(arg, "Sound", "loop")?
    } else {
        0
    };

    let multiple = if let Some(arg) = args.get(index) {
        let flag = value_to_bool(arg, "Sound", "multiple")?;
        index += 1;
        flag
    } else {
        false
    };

    let custom_falloff = if let Some(arg) = args.get(index) {
        Some(value_to_i32(arg, "Sound", "custom_falloff")?).filter(|value| *value != 0)
    } else {
        None
    };

    with_host_context_mut(Ok(Value::Bool(true)), |context| {
        if at_player != 0 {
            let player_id = at_player.wrapping_sub(1);
            let Some(_) = context.player_state(player_id) else {
                return Ok(Value::Bool(false));
            };
            if !context.world.local_players.contains(&player_id)
                && !context
                    .world
                    .physical_viewport_players
                    .borrow()
                    .contains(&player_id)
            {
                return Ok(Value::Bool(true));
            }
        }

        let mut target_id = if let Some(value) = object_value {
            parse_object_reference_argument(value, "Sound", "object")?
        } else {
            None
        };

        if global {
            target_id = None;
        } else if target_id.is_none() {
            target_id = context.object_context().map(|object| object.id());
        }

        if level < 0 {
            return Ok(Value::Bool(true));
        }

        // PrepareFilename turns an extensionless name into `name.wav`, so an
        // empty one asks for ".wav" and matches no sample on any client
        // (C4SoundSystem.cpp:307-320,361-366). Start and stop both find
        // nothing; only the network-safe `true` is left.
        if name.is_empty() {
            return Ok(Value::Bool(true));
        }

        if loop_flag < 0 {
            if !context.stop_synchronous_sound(&name, target_id) {
                context.audio_mut().stop_sound(&name, target_id);
            }
            return Ok(Value::Bool(true));
        }

        // A lethal DoEnergy can synchronously clear an object's effects and
        // then resume the enclosing script callback. Do not let that dead
        // continuation restart a loop its Fx*Stop just stopped. One-shots
        // remain valid for ordinary death callbacks.
        if at_player != 0
            && loop_flag > 0
            && context
                .object_context()
                .is_some_and(|object| !object.alive())
        {
            return Ok(Value::Bool(true));
        }

        let mut volume = level;
        if volume == 0 || volume > 100 {
            volume = 100;
        }
        let volume = volume.clamp(0, 100) as u8;
        let looped = loop_flag > 0;
        // The app host completes NewInstance on the calling game thread. An
        // engine-only fixture has no endpoint and retains the ordered command
        // fallback used by snapshot consumers.
        let _ = context.play_sound(&name, target_id, volume, looped, multiple, custom_falloff);
        Ok(Value::Bool(true))
    })
}

pub(crate) fn sound_level(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnSoundLevel is a bare forward to SoundLevel with no name check
    // (C4Script.cpp:2358-2361), so an empty name is a lookup that finds
    // nothing, not an error.
    let name =
        parse_native_c4_string_argument(args.first(), "SoundLevel", "name")?.unwrap_or_default();

    let level = match args.get(1).unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SoundLevel: expected int for level, got {}",
                other.type_name()
            )));
        }
    };

    let object_arg = args.get(2);

    with_host_context_mut(Ok(Value::Nil), |context| {
        let target_id = if let Some(value) = object_arg {
            parse_object_reference_argument(value, "SoundLevel", "object")?
        } else {
            None
        };

        // FindInst and NewInstance both resolve ".wav" for an empty name and
        // match nothing (C4SoundSystem.cpp:271-286,307-320,361-366).
        if !name.is_empty() && !context.set_synchronous_sound_volume(&name, target_id, level) {
            context.audio_mut().sound_level(&name, target_id, level);
        }
        Ok(Value::Nil)
    })
}

pub(crate) fn enter_audio_context(audio: AudioRegistry) -> AudioContextGuard {
    AUDIO_CONTEXT.with(|cell| {
        let previous = cell.replace(Some(audio));
        AudioContextGuard {
            previous,
            consumed: false,
        }
    })
}

pub(crate) struct AudioContextGuard {
    previous: Option<AudioRegistry>,
    consumed: bool,
}

impl AudioContextGuard {
    pub fn finish(mut self) -> AudioRegistry {
        self.consumed = true;
        AUDIO_CONTEXT.with(|cell| {
            let current = cell.replace(self.previous.take()).unwrap_or_default();
            let mut borrow = cell.borrow_mut();
            let Some(previous) = borrow.as_mut() else {
                return current;
            };
            let inherited_events = current.events.len() >= previous.events.len()
                && current.events[..previous.events.len()] == previous.events[..];
            if inherited_events {
                previous.events = current.events;
            } else {
                previous.events.extend(current.events);
            }
            previous.attached_targets.extend(current.attached_targets);
            previous.next_speech_fallback_id = previous
                .next_speech_fallback_id
                .max(current.next_speech_fallback_id);
            previous.synchronous_host_configured = current.synchronous_host_configured;
            previous.synchronous_host = current.synchronous_host;
            previous.available_samples = current.available_samples;
            previous.available_music = current.available_music;
            previous.music_playlist = current.music_playlist;
            previous.music_level = current.music_level;
            previous.clone()
        })
    }
}

impl Drop for AudioContextGuard {
    fn drop(&mut self) {
        if !self.consumed {
            AUDIO_CONTEXT.with(|cell| {
                cell.replace(self.previous.take());
            });
        }
    }
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct AudioRegistry {
    /// Whether an embedding frontend explicitly supplied its local audio
    /// availability. `false` retains the queued engine-only fixture path;
    /// `true` with no handle mirrors C++'s missing `Application.AudioSystem`.
    synchronous_host_configured: bool,
    /// Non-owning bridge to the app's sole logical C4SoundSystem instance.
    /// This is process-local presentation state and is never serialized.
    synchronous_host: Option<SynchronousSoundHostHandle>,
    /// Filenames admitted by the active C4SoundSystem resource chain. This
    /// is client-local presentation state, not synchronized or save-persisted
    /// state; cloning the registry across callbacks is cheap.
    available_samples: Arc<HashSet<String>>,
    /// Exact basename bytes for each C4MusicFile record in the active
    /// client-side catalog. Keep duplicates: C++ counts records, not unique
    /// filenames.
    available_music: Arc<Vec<Vec<u8>>>,
    /// `None` is C4MusicSystem's default playlist; `Some("")` is the
    /// explicit empty filter produced by script nil/omitted arguments.
    music_playlist: Option<String>,
    /// `Game.iMusicLevel`: save-persisted independently of the local mixer.
    music_level: u8,
    /// Object ids that may still own a frontend sound instance. This is
    /// deliberately coarse because sample resolution and lifetime are local to
    /// the frontend; a false positive only emits a harmless no-op detach.
    attached_targets: HashSet<ObjectId>,
    next_speech_fallback_id: u64,
    pub(crate) events: Vec<AudioCommand>,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct AudioOutcome {
    pub state: AudioRegistry,
    pub events: Vec<AudioCommand>,
}

impl Default for AudioRegistry {
    fn default() -> Self {
        Self {
            synchronous_host_configured: false,
            synchronous_host: None,
            available_samples: Arc::new(HashSet::new()),
            available_music: Arc::new(Vec::new()),
            music_playlist: None,
            music_level: DEFAULT_MUSIC_LEVEL,
            attached_targets: HashSet::new(),
            next_speech_fallback_id: 0,
            events: Vec::new(),
        }
    }
}

impl AudioRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_synchronous_host(&mut self, host: Option<SynchronousSoundHostHandle>) {
        self.synchronous_host_configured = true;
        self.synchronous_host = host;
    }

    pub(crate) fn synchronous_host_configured(&self) -> bool {
        self.synchronous_host_configured
    }

    pub(crate) fn synchronous_host(&self) -> Option<SynchronousSoundHostHandle> {
        self.synchronous_host
    }

    pub(crate) fn set_available_samples<I, S>(&mut self, samples: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.available_samples = Arc::new(
            samples
                .into_iter()
                .map(|sample| normalize_sound_sample_name(sample.as_ref()))
                .collect(),
        );
    }

    pub(crate) fn set_available_music<I, S>(&mut self, tracks: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.available_music = Arc::new(
            tracks
                .into_iter()
                .map(|track| music_script_c_string_bytes(track.as_ref()))
                .collect(),
        );
    }

    pub(crate) fn music_playlist(&self) -> Option<&str> {
        self.music_playlist.as_deref()
    }

    pub(crate) fn restore_music_playlist(&mut self, playlist: Option<String>) {
        self.music_playlist = playlist;
    }

    pub(crate) fn music_level(&self) -> u8 {
        self.music_level
    }

    pub(crate) fn restore_music_level(&mut self, level: u8) -> u8 {
        self.music_level = level.min(DEFAULT_MUSIC_LEVEL);
        self.music_level
    }

    pub(crate) fn set_music_playlist(&mut self, playlist: String, restart: bool) -> i32 {
        let playlist_bytes = music_script_c_string_bytes(&playlist);
        let playlist = clonk_script::c4_string_from_bytes(&playlist_bytes);
        let count = self
            .available_music
            .iter()
            .filter(|track| {
                let filename = music_filename_bytes(track);
                playlist_bytes
                    .split(|byte| *byte == b';')
                    .any(|pattern| music_wildcard_match(pattern, filename))
            })
            .count();
        self.music_playlist = Some(playlist.clone());
        self.events.push(AudioCommand::SetMusicPlaylist {
            playlist: Some(playlist),
            restart,
        });
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    /// Stage message-family speech after the synchronous sample-inventory
    /// gate. The frontend completes `NewInstance` and returns its created or
    /// rejected outcome; ordinary Sound/SoundLevel calls remain fire-and-forget.
    pub(crate) fn try_play_speech(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        fallback: Option<MessageSpec>,
    ) -> (bool, Option<SpeechFallback>) {
        if !sound_sample_available(&self.available_samples, name) {
            return (false, None);
        }
        if let Some(target) = target {
            self.attached_targets.insert(target);
        }
        let fallback = fallback.map(|message| {
            let id = self.next_speech_fallback_id;
            self.next_speech_fallback_id = self.next_speech_fallback_id.wrapping_add(1).max(1);
            SpeechFallback::new(id, message)
        });
        self.events.push(AudioCommand::PlaySpeech {
            name: name.to_string(),
            target,
            fallback: fallback.clone(),
        });
        (true, fallback)
    }

    pub fn play_sound(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: u8,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
    ) {
        if let Some(target) = target {
            self.attached_targets.insert(target);
        }

        self.events.push(AudioCommand::PlaySound {
            name: name.to_string(),
            target,
            volume,
            looped,
            multiple,
            custom_falloff,
            target_position: None,
        });
    }

    pub(crate) fn note_attached_sound(&mut self, target: ObjectId) {
        self.attached_targets.insert(target);
    }

    pub(crate) fn note_detached_sounds(&mut self, target: ObjectId) {
        self.attached_targets.remove(&target);
    }

    pub(crate) fn clear_sound_instances(&mut self) {
        self.attached_targets.clear();
        self.events.retain(|event| {
            matches!(
                event,
                AudioCommand::PlayMusic { .. }
                    | AudioCommand::StopMusic
                    | AudioCommand::SetMusicLevel { .. }
                    | AudioCommand::SetMusicPlaylist { .. }
            )
        });
    }

    pub(crate) fn detach_object_sounds(&mut self, target: ObjectId, position: Vector2) {
        let was_attached = self.attached_targets.remove(&target);
        if was_attached {
            self.events
                .push(AudioCommand::DetachObjectSounds { target, position });
        }
    }

    pub fn stop_sound(&mut self, name: &str, target: Option<ObjectId>) {
        self.events.push(AudioCommand::StopSound {
            name: name.to_string(),
            target,
        });
    }

    pub(crate) fn sound_level(&mut self, name: &str, target: Option<ObjectId>, volume: i32) {
        if volume <= 0 {
            self.stop_sound(name, target);
            return;
        }
        if let Some(target) = target {
            self.attached_targets.insert(target);
        }
        self.events.push(AudioCommand::SetSoundVolume {
            name: name.to_string(),
            target,
            volume,
        });
    }

    pub fn take_events(&mut self) -> Vec<AudioCommand> {
        mem::take(&mut self.events)
    }

    pub fn play_music(&mut self, name: String, looped: bool) {
        self.events.push(AudioCommand::PlayMusic { name, looped });
    }

    pub fn stop_music(&mut self) {
        self.events.push(AudioCommand::StopMusic);
    }

    pub fn set_music_level(&mut self, level: i32) -> u8 {
        let level = level.clamp(0, 100) as u8;
        self.music_level = level;
        self.events.push(AudioCommand::SetMusicLevel { level });
        self.music_level
    }
}

fn music_script_c_string_bytes(value: &str) -> Vec<u8> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(end);
    }
    bytes
}

fn music_filename_bytes(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\'))
        .map_or(path, |separator| &path[separator + 1..])
}

fn music_wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            pattern_index = saved_pattern;
            value_index = saved_value.saturating_add(1);
            backtrack_value = Some(value_index);
        } else {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

impl Default for AudioOutcome {
    fn default() -> Self {
        Self {
            state: AudioRegistry::new(),
            events: Vec::new(),
        }
    }
}

pub(crate) fn normalize_sound_name(name: &str) -> String {
    let mut bytes = clonk_script::c4_string_bytes(name);
    bytes
        .iter_mut()
        .for_each(|byte| *byte = c4_char_capital(*byte));
    clonk_script::c4_string_from_bytes(&bytes)
}

fn normalize_sound_sample_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// `C4SoundSystem::PrepareFilename` plus StdFile's ASCII-insensitive
/// WildcardMatch. Extensionless requests resolve only `.wav`, and legacy
/// replaces every `*` with the one-character `?` wildcard.
pub(crate) fn sound_sample_available(samples: &HashSet<String>, name: &str) -> bool {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let has_extension = file_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| !extension.is_empty());
    let mut pattern = if has_extension {
        name.to_ascii_lowercase()
    } else {
        format!("{}.wav", name.to_ascii_lowercase())
    };
    pattern = pattern.replace('*', "?");

    if pattern.contains('?') {
        samples
            .iter()
            .any(|sample| sound_filename_matches(&pattern, sample))
    } else {
        samples.contains(&pattern)
    }
}

fn sound_filename_matches(pattern: &str, file_name: &str) -> bool {
    let pattern = pattern.as_bytes();
    let file_name = file_name.as_bytes();
    pattern.len() == file_name.len()
        && pattern
            .iter()
            .zip(file_name)
            .all(|(expected, actual)| *expected == b'?' || expected.eq_ignore_ascii_case(actual))
}
