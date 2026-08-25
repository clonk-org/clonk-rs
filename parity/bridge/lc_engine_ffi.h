#pragma once

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LcEngineRecorderHandle LcEngineRecorderHandle;
typedef struct LcEnginePlaybackHandle LcEnginePlaybackHandle;
typedef struct LcEngineRuntimeHandle LcEngineRuntimeHandle;

typedef struct LcEngineEffectSnapshot {
    const char *name;
   int32_t priority;
    int32_t interval;
    int32_t timer;
} LcEngineEffectSnapshot;

typedef struct LcEngineParticleSnapshot {
    const char *definition_id;
   float x;
    float y;
    float xdir;
    float ydir;
    int32_t life;
    float parameter_a;
    int32_t parameter_b;
    int32_t layer;
    bool has_owner;
    uint64_t owner_id;
} LcEngineParticleSnapshot;

typedef struct LcEngineObjectVertexSnapshot {
    int32_t x;
    int32_t y;
    uint32_t cnat;
    int32_t friction;
} LcEngineObjectVertexSnapshot;

typedef struct LcEngineObjectSnapshot {
    uint64_t id;
    const char *definition_id;
    int32_t position_x;
    int32_t position_y;
    int32_t velocity_x;
    int32_t velocity_y;
    int32_t rotation;
    int32_t fixed_position_x;
    int32_t fixed_position_y;
    int32_t fixed_velocity_x;
    int32_t fixed_velocity_y;
    int32_t fixed_rotation;
    bool mobile;
    bool in_liquid;
    int32_t object_timer;
    int32_t rotation_velocity;
    int32_t energy;
    int32_t construction;
    int32_t damage;
    int32_t magic_energy;
    int32_t magic_capacity;
    int32_t owner;
    int32_t category;
    bool crew_member;
    bool alive;
    const char *action_name;
    int32_t action_phase;
    int32_t action_ticks;
    int32_t action_data;
    int32_t direction;
    int32_t command_direction;
    const LcEngineEffectSnapshot *effects;
    size_t effect_count;
    const LcEngineObjectVertexSnapshot *vertices;
    size_t vertex_count;
    bool has_container;
    uint64_t container_id;
    const uint64_t *contents;
    size_t contents_count;
    bool has_base_graphics;
    const char *base_definition_id;
    const char *base_graphics_name;
    uint32_t base_blit_mode;
    bool has_draw_transform;
    float draw_scale_x;
    float draw_scale_y;
    float draw_offset_x;
    float draw_offset_y;
} LcEngineObjectSnapshot;

typedef struct LcEngineCrewSelectionSnapshot {
    int32_t owner;
    const uint64_t *selected;
    size_t selected_count;
    bool has_cursor;
    uint64_t cursor;
} LcEngineCrewSelectionSnapshot;

typedef struct LcEngineCrewRoleAssignment {
    uint64_t object_id;
    const char *role;
} LcEngineCrewRoleAssignment;

typedef struct LcEngineCrewRoleSnapshot {
    int32_t owner;
    const LcEngineCrewRoleAssignment *assignments;
    size_t assignment_count;
} LcEngineCrewRoleSnapshot;

typedef struct LcEngineHudPlayerSnapshot {
    int32_t owner;
    const uint64_t *crew;
    size_t crew_count;
    bool has_focus;
    uint64_t focus_object;
    bool eliminated;
    int32_t wealth;
    int32_t score;
} LcEngineHudPlayerSnapshot;

typedef struct LcEngineSurfaceSnapshot {
    const char *label;
   int32_t width;
    int32_t height;
    uint64_t hash;
} LcEngineSurfaceSnapshot;

typedef struct LcEnginePathWaypoint {
   int32_t x;
    int32_t y;
    bool has_transfer_target;
    uint64_t transfer_target;
} LcEnginePathWaypoint;

typedef struct LcEnginePathSlice {
    bool found;
   int32_t length;
    const LcEnginePathWaypoint *waypoints;
    size_t waypoint_count;
} LcEnginePathSlice;

typedef struct LcEngineNetworkPacketSnapshot {
    uint8_t direction; // 0 = inbound, 1 = outbound
    uint8_t status;
    uint16_t reserved;
    uint32_t size;
    uint64_t hash;
    int32_t client_id;
    uint32_t connection_id;
} LcEngineNetworkPacketSnapshot;

LcEngineRecorderHandle *lc_engine_recorder_new(void);
void lc_engine_recorder_clear(LcEngineRecorderHandle *handle);
void lc_engine_recorder_record(
    LcEngineRecorderHandle *handle,
    uint64_t frame,
    const LcEngineObjectSnapshot *objects,
    size_t object_count,
    const LcEngineEffectSnapshot *global_effects,
    size_t global_effect_count,
    const LcEngineParticleSnapshot *particles,
    size_t particle_count,
    const LcEngineCrewSelectionSnapshot *crew_selection,
    size_t crew_selection_count,
    const LcEngineCrewRoleSnapshot *crew_roles,
    size_t crew_role_count,
    const int32_t *known_crew_owners,
    size_t known_crew_owner_count,
    const int32_t *eliminated_crew_owners,
    size_t eliminated_crew_owner_count,
    const LcEngineHudPlayerSnapshot *hud_players,
    size_t hud_player_count,
    const LcEngineSurfaceSnapshot *surfaces,
    size_t surface_count,
    const LcEngineNetworkPacketSnapshot *network_packets,
    size_t network_packet_count,
    const char *const *controls,
    size_t control_count);
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
    const LcEngineParticleSnapshot *particles,
    size_t particle_count,
    const LcEngineCrewSelectionSnapshot *crew_selection,
    size_t crew_selection_count,
    const LcEngineCrewRoleSnapshot *crew_roles,
    size_t crew_role_count,
    const LcEngineHudPlayerSnapshot *hud_players,
    size_t hud_player_count,
    const LcEngineSurfaceSnapshot *surfaces,
    size_t surface_count,
    const LcEngineNetworkPacketSnapshot *network_packets,
    size_t network_packet_count,
    const char *const *controls,
    size_t control_count,
    const int32_t *known_crew_owners,
    size_t known_crew_owner_count,
    const int32_t *eliminated_crew_owners,
    size_t eliminated_crew_owner_count,
    char **error_message);
bool lc_engine_playback_finish(LcEnginePlaybackHandle *handle, char **error_message);
void lc_engine_playback_free(LcEnginePlaybackHandle *handle);

void lc_engine_string_free(char *value);

LcEngineRuntimeHandle *lc_engine_runtime_new(void);
void lc_engine_runtime_free(LcEngineRuntimeHandle *handle);
bool lc_engine_runtime_load_scenario(
    LcEngineRuntimeHandle *handle,
    const char *path,
    uint64_t seed,
    char **error_message);
bool lc_engine_runtime_record_control_ini(
    LcEngineRuntimeHandle *handle,
    uint64_t frame,
    const char *ini_data,
    char **error_message);
bool lc_engine_runtime_reset(LcEngineRuntimeHandle *handle, char **error_message);
bool lc_engine_runtime_advance_to_frame(
    LcEngineRuntimeHandle *handle,
    uint64_t frame,
    char **error_message);
bool lc_engine_runtime_step(LcEngineRuntimeHandle *handle, char **error_message);
uint64_t lc_engine_runtime_current_frame(const LcEngineRuntimeHandle *handle);

typedef struct LcEngineRuntimeObjectState {
    uint64_t id;
    const char *definition_id;
    int32_t position_x;
    int32_t position_y;
    int32_t velocity_x;
    int32_t velocity_y;
    int32_t rotation;
    int32_t fixed_position_x;
    int32_t fixed_position_y;
    int32_t fixed_velocity_x;
    int32_t fixed_velocity_y;
    int32_t fixed_rotation;
    bool mobile;
    bool in_liquid;
    int32_t object_timer;
    int32_t rotation_velocity;
    int32_t energy;
    int32_t construction;
    int32_t damage;
    int32_t owner;
    int32_t category;
    bool crew_member;
    bool alive;
    int32_t status;
    const char *action_name;
    int32_t action_phase;
    int32_t action_ticks;
    int32_t action_data;
    int32_t direction;
    int32_t command_direction;
    bool has_container;
    uint64_t container_id;
    const uint64_t *contents;
    size_t contents_len;
} LcEngineRuntimeObjectState;

typedef struct LcEngineRuntimeObjectStateSlice {
    uint64_t frame;
    const LcEngineRuntimeObjectState *objects;
    size_t object_count;
} LcEngineRuntimeObjectStateSlice;

typedef struct LcEngineRuntimeEnvironmentState {
    int32_t wind;
    int32_t wind_variation;
    uint32_t wind_period;
    int32_t temperature;
    int32_t climate;
    int32_t temperature_variation;
    uint32_t temperature_period;
    uint32_t temperature_phase;
    uint16_t time_of_day;
    int16_t time_speed;
    int32_t precipitation;
    bool has_sky_color;
    uint8_t sky_color_r;
    uint8_t sky_color_g;
    uint8_t sky_color_b;
} LcEngineRuntimeEnvironmentState;

typedef struct LcEngineRuntimeObjectStateArray LcEngineRuntimeObjectStateArray;
typedef struct LcEngineRuntimeLandscapeArray LcEngineRuntimeLandscapeArray;
typedef struct LcEngineRuntimePathResult LcEngineRuntimePathResult;

typedef struct LcEngineRuntimeLandscapeSlice {
    uint32_t width;
    const int32_t *heights;
} LcEngineRuntimeLandscapeSlice;

LcEngineRuntimeObjectStateArray *lc_engine_runtime_export_object_states(
    LcEngineRuntimeHandle *handle,
    char **error_message);
LcEngineRuntimeObjectStateSlice lc_engine_runtime_object_states_slice(
    const LcEngineRuntimeObjectStateArray *buffer);
void lc_engine_runtime_object_states_free(
    LcEngineRuntimeObjectStateArray *buffer);

bool lc_engine_runtime_compare_snapshot(
    LcEngineRuntimeHandle *handle,
    uint64_t frame,
    const LcEngineObjectSnapshot *objects,
    size_t object_count,
    const LcEngineEffectSnapshot *global_effects,
    size_t global_effect_count,
    const LcEngineParticleSnapshot *particles,
    size_t particle_count,
    const LcEngineCrewSelectionSnapshot *crew_selection,
    size_t crew_selection_count,
    const LcEngineCrewRoleSnapshot *crew_roles,
    size_t crew_role_count,
    const int32_t *known_crew_owners,
    size_t known_crew_owner_count,
    const int32_t *eliminated_crew_owners,
    size_t eliminated_crew_owner_count,
    const LcEngineHudPlayerSnapshot *hud_players,
    size_t hud_player_count,
    const LcEngineSurfaceSnapshot *surfaces,
    size_t surface_count,
    const LcEngineNetworkPacketSnapshot *network_packets,
    size_t network_packet_count,
    const char *const *controls,
    size_t control_count,
    uint32_t rng_hold,
    int32_t rng_count,
    char **error_message);
char *lc_engine_runtime_export_snapshot_json(
    LcEngineRuntimeHandle *handle,
    char **error_message);
char *lc_engine_runtime_export_state_json(
    LcEngineRuntimeHandle *handle,
    char **error_message);
bool lc_engine_runtime_import_state_json(
    LcEngineRuntimeHandle *handle,
    const char *json,
    char **error_message);
bool lc_engine_runtime_export_environment(
    const LcEngineRuntimeHandle *handle,
    LcEngineRuntimeEnvironmentState *out,
    char **error_message);
LcEngineRuntimeLandscapeArray *lc_engine_runtime_export_landscape(
    const LcEngineRuntimeHandle *handle,
    char **error_message);
LcEngineRuntimeLandscapeSlice lc_engine_runtime_landscape_slice(
    const LcEngineRuntimeLandscapeArray *buffer);
void lc_engine_runtime_landscape_free(LcEngineRuntimeLandscapeArray *buffer);
LcEngineRuntimePathResult *lc_engine_runtime_find_path(
    const LcEngineRuntimeHandle *handle,
    int32_t from_x,
    int32_t from_y,
    int32_t to_x,
    int32_t to_y,
    bool transfer_zones_enabled,
    int32_t level,
    char **error_message);
LcEnginePathSlice lc_engine_runtime_path_slice(
    const LcEngineRuntimePathResult *buffer);
void lc_engine_runtime_path_free(LcEngineRuntimePathResult *buffer);

#ifdef __cplusplus
}
#endif
