#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AudioSystemHandle AudioSystemHandle;
typedef struct AudioSoundHandle AudioSoundHandle;
typedef struct AudioMusicHandle AudioMusicHandle;
typedef struct AudioChannelHandle AudioChannelHandle;

typedef void (*LcAudioChannelFinishedCallback)(int channel, void *user_data);

AudioSystemHandle *lc_audio_system_new(uint32_t max_channels);
void lc_audio_system_free(AudioSystemHandle *handle);
void lc_audio_system_register_channel_finished_callback(AudioSystemHandle *handle, LcAudioChannelFinishedCallback callback, void *user_data);

AudioSoundHandle *lc_audio_system_load_sound(AudioSystemHandle *handle, const uint8_t *data, size_t len);
void lc_audio_sound_free(AudioSoundHandle *handle);
uint32_t lc_audio_sound_duration_ms(AudioSoundHandle *handle);

AudioMusicHandle *lc_audio_system_load_music(AudioSystemHandle *handle, const uint8_t *data, size_t len);
void lc_audio_music_free(AudioMusicHandle *handle);

AudioChannelHandle *lc_audio_system_play_sound(AudioSystemHandle *handle, AudioSoundHandle *sound, bool looped);
void lc_audio_channel_free(AudioChannelHandle *handle);
bool lc_audio_channel_is_playing(AudioChannelHandle *handle);
void lc_audio_channel_set_volume_pan(AudioChannelHandle *handle, float volume, float pan);
void lc_audio_channel_stop(AudioChannelHandle *handle);

bool lc_audio_system_play_music(AudioSystemHandle *handle, AudioMusicHandle *music, bool looped);
void lc_audio_system_halt_music(AudioSystemHandle *handle);
bool lc_audio_system_music_is_playing(AudioSystemHandle *handle);
void lc_audio_system_music_set_volume(AudioSystemHandle *handle, float volume);
bool lc_audio_system_music_fade_out(AudioSystemHandle *handle, uint32_t duration_ms);

#ifdef __cplusplus
}
#endif
