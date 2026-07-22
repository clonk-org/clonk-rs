use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use crate::mixer::{AudioMixer, AudioSystem, ChannelId, MusicHandle, SoundHandle};

pub struct AudioSystemHandle(AudioSystem);

pub struct AudioSoundHandle(SoundHandle);

pub struct AudioMusicHandle(MusicHandle);

pub struct AudioChannelHandle {
    mixer: Arc<AudioMixer>,
    channel: ChannelId,
}

#[no_mangle]
pub extern "C" fn lc_audio_system_new(max_channels: u32) -> *mut AudioSystemHandle {
    match AudioSystem::new(max_channels as usize) {
        Ok(system) => Box::into_raw(Box::new(AudioSystemHandle(system))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_system_free(handle: *mut AudioSystemHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_system_register_channel_finished_callback(
    handle: *mut AudioSystemHandle,
    callback: Option<extern "C" fn(i32, *mut c_void)>,
    user_data: *mut c_void,
) {
    if handle.is_null() {
        return;
    }
    let system = unsafe { &mut *handle };
    system
        .0
        .set_channel_finished_callback_ffi(callback, user_data);
}

#[no_mangle]
pub extern "C" fn lc_audio_system_load_sound(
    handle: *mut AudioSystemHandle,
    data: *const u8,
    len: usize,
) -> *mut AudioSoundHandle {
    if handle.is_null() || data.is_null() {
        return ptr::null_mut();
    }
    let system = unsafe { &mut *handle };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    match system.0.load_sound(bytes) {
        Ok(sound) => Box::into_raw(Box::new(AudioSoundHandle(sound))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_sound_free(handle: *mut AudioSoundHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        // Dropping the handle unloads the sound once no other clone uses it.
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_sound_duration_ms(handle: *mut AudioSoundHandle) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let sound = unsafe { &mut *handle };
    sound.0.duration_ms().unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn lc_audio_system_load_music(
    handle: *mut AudioSystemHandle,
    data: *const u8,
    len: usize,
) -> *mut AudioMusicHandle {
    if handle.is_null() || data.is_null() {
        return ptr::null_mut();
    }
    let system = unsafe { &mut *handle };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    match system.0.load_music(bytes) {
        Ok(music) => Box::into_raw(Box::new(AudioMusicHandle(music))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_music_free(handle: *mut AudioMusicHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        // Dropping the handle unloads the music once no other clone uses it.
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_system_play_sound(
    handle: *mut AudioSystemHandle,
    sound: *mut AudioSoundHandle,
    looped: bool,
) -> *mut AudioChannelHandle {
    if handle.is_null() || sound.is_null() {
        return ptr::null_mut();
    }
    let system = unsafe { &mut *handle };
    let sound_handle = unsafe { &mut *sound };
    match system.0.play_sound(&sound_handle.0, looped) {
        Ok(channel) => Box::into_raw(Box::new(AudioChannelHandle {
            mixer: Arc::clone(system.0.mixer()),
            channel,
        })),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_channel_free(handle: *mut AudioChannelHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let boxed = Box::from_raw(handle);
        boxed.mixer.halt_channel(boxed.channel);
    }
}

#[no_mangle]
pub extern "C" fn lc_audio_channel_is_playing(handle: *mut AudioChannelHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let channel = unsafe { &mut *handle };
    channel.mixer.channel_is_playing(channel.channel)
}

#[no_mangle]
pub extern "C" fn lc_audio_channel_set_volume_pan(
    handle: *mut AudioChannelHandle,
    volume: f32,
    pan: f32,
) {
    if handle.is_null() {
        return;
    }
    let channel = unsafe { &mut *handle };
    channel
        .mixer
        .channel_set_volume_and_pan(channel.channel, volume, pan);
}

#[no_mangle]
pub extern "C" fn lc_audio_channel_stop(handle: *mut AudioChannelHandle) {
    if handle.is_null() {
        return;
    }
    let channel = unsafe { &mut *handle };
    channel.mixer.halt_channel(channel.channel);
}

#[no_mangle]
pub extern "C" fn lc_audio_system_play_music(
    handle: *mut AudioSystemHandle,
    music: *mut AudioMusicHandle,
    looped: bool,
) -> bool {
    if handle.is_null() || music.is_null() {
        return false;
    }
    let system = unsafe { &mut *handle };
    let music_handle = unsafe { &mut *music };
    system.0.play_music(&music_handle.0, looped).is_ok()
}

#[no_mangle]
pub extern "C" fn lc_audio_system_halt_music(handle: *mut AudioSystemHandle) {
    if handle.is_null() {
        return;
    }
    let system = unsafe { &mut *handle };
    system.0.halt_music();
}

#[no_mangle]
pub extern "C" fn lc_audio_system_music_is_playing(handle: *mut AudioSystemHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let system = unsafe { &mut *handle };
    system.0.music_is_playing()
}

#[no_mangle]
pub extern "C" fn lc_audio_system_music_set_volume(handle: *mut AudioSystemHandle, volume: f32) {
    if handle.is_null() {
        return;
    }
    let system = unsafe { &mut *handle };
    system.0.music_set_volume(volume);
}

#[no_mangle]
pub extern "C" fn lc_audio_system_music_fade_out(
    handle: *mut AudioSystemHandle,
    duration_ms: u32,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let system = unsafe { &mut *handle };
    system.0.music_fade_out(duration_ms)
}
