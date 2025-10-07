#pragma once

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LcEngineRecorderHandle LcEngineRecorderHandle;
typedef struct LcEnginePlaybackHandle LcEnginePlaybackHandle;

typedef struct LcEngineEffectSnapshot {
    const char *name;
    int32_t priority;
    int32_t interval;
    int32_t timer;
} LcEngineEffectSnapshot;

typedef struct LcEngineObjectSnapshot {
    uint64_t id;
    const char *definition_id;
    int32_t position_x;
    int32_t position_y;
    int32_t velocity_x;
    int32_t velocity_y;
    int32_t energy;
    int32_t owner;
    bool crew_member;
    const char *action_name;
    int32_t action_phase;
    const LcEngineEffectSnapshot *effects;
    size_t effect_count;
} LcEngineObjectSnapshot;

LcEngineRecorderHandle *lc_engine_recorder_new(void);
void lc_engine_recorder_clear(LcEngineRecorderHandle *handle);
void lc_engine_recorder_record(
    LcEngineRecorderHandle *handle,
    uint64_t frame,
    const LcEngineObjectSnapshot *objects,
    size_t object_count,
    const LcEngineEffectSnapshot *global_effects,
    size_t global_effect_count);
char *lc_engine_recorder_export_json(LcEngineRecorderHandle *handle);
void lc_engine_recorder_free(LcEngineRecorderHandle *handle);

LcEnginePlaybackHandle *lc_engine_playback_from_json(const char *json, char **error_message);
bool lc_engine_playback_compare(
    LcEnginePlaybackHandle *handle,
    uint64_t frame,
    const LcEngineObjectSnapshot *objects,
    size_t object_count,
    const LcEngineEffectSnapshot *global_effects,
    size_t global_effect_count,
    char **error_message);
bool lc_engine_playback_finish(LcEnginePlaybackHandle *handle, char **error_message);
void lc_engine_playback_free(LcEnginePlaybackHandle *handle);

void lc_engine_string_free(char *value);

#ifdef __cplusplus
}
#endif
