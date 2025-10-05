#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LcEngineRecorderHandle LcEngineRecorderHandle;
typedef struct LcEnginePlaybackHandle LcEnginePlaybackHandle;

typedef struct LcEngineObjectSnapshot {
    uint64_t id;
    const char *definition_id;
    int32_t position_x;
    int32_t position_y;
    int32_t velocity_x;
    int32_t velocity_y;
    int32_t energy;
} LcEngineObjectSnapshot;

LcEngineRecorderHandle *lc_engine_recorder_new(void);
void lc_engine_recorder_clear(LcEngineRecorderHandle *handle);
void lc_engine_recorder_record(LcEngineRecorderHandle *handle, uint64_t frame, const LcEngineObjectSnapshot *objects, size_t len);
char *lc_engine_recorder_export_json(LcEngineRecorderHandle *handle);
void lc_engine_recorder_free(LcEngineRecorderHandle *handle);

LcEnginePlaybackHandle *lc_engine_playback_from_json(const char *json, char **error_message);
bool lc_engine_playback_compare(LcEnginePlaybackHandle *handle, uint64_t frame, const LcEngineObjectSnapshot *objects, size_t len, char **error_message);
bool lc_engine_playback_finish(LcEnginePlaybackHandle *handle, char **error_message);
void lc_engine_playback_free(LcEnginePlaybackHandle *handle);

void lc_engine_string_free(char *value);

#ifdef __cplusplus
}
#endif
