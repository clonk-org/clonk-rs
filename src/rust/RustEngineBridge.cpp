#include "C4Random.h"
#include "RustEngineBridge.h"

#ifdef USE_RUST_ENGINE_VALIDATION

#include "lc_engine_ffi.h"

#include <C4Include.h>
#include <C4Application.h>
#include <C4Control.h>
#include <C4Game.h>
#include <C4GraphicsSystem.h>
#include <C4Log.h>
#include <C4Object.h>
#include <C4ObjectList.h>
#include <C4Effects.h>
#include <C4Particles.h>
#include <C4Player.h>
#include <C4Material.h>
#include <C4Weather.h>
#include <C4Viewport.h>
#include <C4Wrappers.h>
#include <StdCompiler.h>

#include <Fixed.h>

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <cstdint>
#include <iterator>
#include <memory>
#include <mutex>
#include <exception>
#include <map>
#include <set>
#include <utility>
#include <format>
#include <string>
#include <string_view>
#include <vector>
#include <limits>
#include <unordered_map>
#include <unordered_set>

namespace {

constexpr uint64_t kFnvOffsetBasis = 1469598103934665603ull;
constexpr uint64_t kFnvPrime = 1099511628211ull;

struct RecorderDeleter {
    void operator()(LcEngineRecorderHandle *handle) const {
        if (handle) {
            lc_engine_recorder_free(handle);
        }
    }
};

struct PlaybackDeleter {
    void operator()(LcEnginePlaybackHandle *handle) const {
        if (handle) {
            lc_engine_playback_free(handle);
        }
    }
};

struct RuntimeDeleter {
    void operator()(LcEngineRuntimeHandle *handle) const {
        if (handle) {
            lc_engine_runtime_free(handle);
        }
    }
};

using RecorderPtr = std::unique_ptr<LcEngineRecorderHandle, RecorderDeleter>;
using PlaybackPtr = std::unique_ptr<LcEnginePlaybackHandle, PlaybackDeleter>;
using RuntimePtr = std::unique_ptr<LcEngineRuntimeHandle, RuntimeDeleter>;
using RustStringPtr = std::unique_ptr<char, decltype(&lc_engine_string_free)>;

// Defined later in this namespace; declared here for the helpers above them.
void LogWarning(const std::string &message);
void ResetAuthoritativeLandscapeCache();
int32_t DetermineSurfaceHeight(C4Game &game, int32_t x);
bool AdjustLandscapeColumn(C4Game &game, int32_t x, int32_t current_height, int32_t target_height);
RustStringPtr MakeString(char *raw);
extern uint64_t g_last_missing_warning_frame;
extern uint64_t g_last_container_warning_frame;
extern std::vector<int32_t> g_authoritative_landscape_heights;
extern RuntimePtr g_runtime;
extern bool g_runtime_disabled;


class SurfaceLock {
public:
    explicit SurfaceLock(C4Surface *surface)
        : surface(surface)
        , locked(false) {
        if (surface) {
            locked = surface->Lock();
        }
    }

    ~SurfaceLock() {
        if (locked && surface) {
            surface->Unlock();
        }
    }

    bool IsLocked() const {
        return locked;
    }

private:
    C4Surface *surface;
    bool locked;
};

uint64_t HashSurfaceRegion(
    C4Surface &surface,
    int32_t origin_x,
    int32_t origin_y,
    int32_t width,
    int32_t height,
    float scale) {
    uint64_t hash = kFnvOffsetBasis;
    for (int32_t y = 0; y < height; ++y) {
        for (int32_t x = 0; x < width; ++x) {
            const uint32_t pixel =
                surface.GetPixDw(origin_x + x, origin_y + y, false, scale);
            hash ^= static_cast<uint64_t>(pixel);
            hash *= kFnvPrime;
        }
    }
    return hash;
}

uint64_t HashNetworkPacket(uint8_t status, const uint8_t *data, size_t size) {
    uint64_t hash = kFnvOffsetBasis;
    hash ^= static_cast<uint64_t>(status);
    hash *= kFnvPrime;
    hash ^= static_cast<uint64_t>(size);
    hash *= kFnvPrime;
    if (data && size > 0) {
        for (size_t index = 0; index < size; ++index) {
            hash ^= static_cast<uint64_t>(data[index]);
            hash *= kFnvPrime;
        }
    } else {
        hash ^= 0;
        hash *= kFnvPrime;
    }
    return hash;
}

int32_t ClampActionTicks(int32_t ticks) {
    if (ticks < 0) {
        return 0;
    }
    return ticks;
}

C4Def *FindDefinitionForRuntimeIdentifier(const char *identifier) {
    if (!identifier || !*identifier) {
        return nullptr;
    }

    std::string_view view(identifier);
    if (LooksLikeID(view)) {
        if (C4Def *by_id = Game.Defs.ID2Def(C4Id(view))) {
            return by_id;
        }
    }

    for (const auto &def_entry : Game.Defs) {
        C4Def *def = def_entry.get();
        if (!def) {
            continue;
        }
        const char *name = def->GetName();
        if (name && SEqualNoCase(name, identifier)) {
            return def;
        }
    }

    return nullptr;
}

int32_t RuntimeObjectNumber(uint64_t id) {
    if (id == 0 || id > static_cast<uint64_t>(std::numeric_limits<int32_t>::max())) {
        return -1;
    }
    return static_cast<int32_t>(id);
}

void EnsureObjectStatusMatchesRuntimeState(C4Object &object, int32_t desired_status) {
    if (desired_status == C4OS_INACTIVE) {
        if (object.Status == C4OS_NORMAL) {
            object.StatusDeactivate(false);
        }
    } else {
        if (object.Status == C4OS_INACTIVE) {
            object.StatusActivate();
        }
    }
}

C4Object *FindObjectByRuntimeId(C4Game &game, uint64_t id) {
    const int32_t number = RuntimeObjectNumber(id);
    if (number <= 0) {
        return nullptr;
    }
    if (C4Object *object = game.Objects.SafeObjectPointer(number)) {
        return object;
    }
    return game.Objects.InactiveObjects.ObjectPointer(number);
}

bool WouldIntroduceContainerCycle(C4Object &object, C4Object *candidate) {
    if (!candidate) {
        return false;
    }
    if (&object == candidate) {
        return true;
    }
    for (C4Object *ancestor = candidate->Contained; ancestor; ancestor = ancestor->Contained) {
        if (ancestor == &object) {
            return true;
        }
    }
    return false;
}

static int32_t NormalizeCategory(int32_t desired, int32_t fallback) {
    if (desired & C4D_SortLimit) {
        return desired;
    }
    int32_t sort_bits = fallback & C4D_SortLimit;
    if (!sort_bits) {
        sort_bits = C4D_StaticBack;
    }
    return (desired & ~C4D_SortLimit) | sort_bits;
}

static C4Fixed C4FixedFromRaw(int32_t raw) {
    C4Fixed value;
    value.val = raw;
    return value;
}

C4Object *CreateRuntimeObject(
    C4Game &game,
    const LcEngineRuntimeObjectState &state,
    uint64_t frame) {
    if (state.status == C4OS_DELETED) {
        return nullptr;
    }

    C4Def *definition = FindDefinitionForRuntimeIdentifier(state.definition_id);
    if (!definition) {
        LogWarning(std::format(
            "Rust runtime could not resolve definition \"{}\" for object {} on frame {}",
            state.definition_id ? state.definition_id : "<unknown>",
            state.id,
            frame));
        return nullptr;
    }

    const int32_t number = RuntimeObjectNumber(state.id);
    if (number <= 0) {
        LogWarning(std::format(
            "Rust runtime reported unsupported object id {} on frame {}",
            state.id,
            frame));
        return nullptr;
    }

    if (game.Objects.ObjectPointer(number) || game.Objects.InactiveObjects.ObjectPointer(number)) {
        return nullptr;
    }

    auto object = std::make_unique<C4Object>();
    if (!object->Init(
            definition,
            nullptr,
            state.owner,
            nullptr,
            state.position_x,
            state.position_y,
            state.rotation,
            C4FixedFromRaw(state.fixed_velocity_x),
            C4FixedFromRaw(state.fixed_velocity_y),
            C4FixedFromRaw(state.rotation_velocity),
            state.owner)) {
        LogWarning(std::format(
            "Rust runtime mirror failed to initialise object \"{}\" ({}) on frame {}",
            state.definition_id ? state.definition_id : "<unknown>",
            state.id,
            frame));
        return nullptr;
    }

    const int32_t normalized_category = NormalizeCategory(state.category, definition->Category);
    object->SetCategory(normalized_category);
    object->fix_x = C4FixedFromRaw(state.fixed_position_x);
    object->fix_y = C4FixedFromRaw(state.fixed_position_y);
    object->fix_r = C4FixedFromRaw(state.fixed_rotation);
    object->r = state.rotation;
    object->xdir = C4FixedFromRaw(state.fixed_velocity_x);
    object->ydir = C4FixedFromRaw(state.fixed_velocity_y);
    object->rdir = C4FixedFromRaw(state.rotation_velocity);

    object->DoCon(
        std::clamp(state.construction, 0, FullCon) - object->GetCon(), true, true);
    object->UpdateMass();
    object->UpdateFace(true);
    object->SetOCF();
    object->UpdatePos();

    object->Number = number;
    if (game.ObjectEnumerationIndex < number) {
        game.ObjectEnumerationIndex = number;
    }

    C4Object *raw = object.get();
    if (!game.Objects.Add(raw)) {
        LogWarning(std::format(
            "Rust runtime mirror could not attach object \"{}\" ({}) to game state on frame {}",
            state.definition_id ? state.definition_id : "<unknown>",
            state.id,
            frame));
        return nullptr;
    }

    object.release();
    return raw;
}

void ApplyRuntimeObjectStateToC4Object(
    C4Object &object,
    const LcEngineRuntimeObjectState &state) {
    if (state.status == C4OS_DELETED) {
        return;
    }

    EnsureObjectStatusMatchesRuntimeState(object, state.status);

    const int32_t pos_x = state.position_x;
    const int32_t pos_y = state.position_y;
    const int32_t vel_x = state.velocity_x;
    const int32_t vel_y = state.velocity_y;

    object.old_x = object.x;
    object.old_y = object.y;
    object.x = pos_x;
    object.y = pos_y;
    object.r = state.rotation;
    object.fix_x = C4FixedFromRaw(state.fixed_position_x);
    object.fix_y = C4FixedFromRaw(state.fixed_position_y);
    object.fix_r = C4FixedFromRaw(state.fixed_rotation);
    object.motion_x = vel_x;
    object.motion_y = vel_y;
    object.xdir = C4FixedFromRaw(state.fixed_velocity_x);
    object.ydir = C4FixedFromRaw(state.fixed_velocity_y);
    object.rdir = C4FixedFromRaw(state.rotation_velocity);

    object.Energy = state.energy;
    object.Damage = std::max<int32_t>(state.damage, 0);
    if (object.Owner != state.owner) {
        object.SetOwner(state.owner);
    }
    object.DoCon(
        std::clamp(state.construction, 0, FullCon) - object.GetCon(), true, true);
    object.UpdateMass();
    object.UpdateFace(true);
    object.SetOCF();

    const int32_t normalized_category = NormalizeCategory(state.category, object.Category);
    if (object.Category != normalized_category) {
        object.SetCategory(normalized_category);
    }

    object.SetAlive(state.alive);

    const char *desired_action = state.action_name ? state.action_name : "";
    const char *current_action = object.Action.Name;
    if (!current_action) {
        current_action = "";
    }
    if (std::strcmp(current_action, desired_action) != 0) {
        object.SetActionByName(desired_action, nullptr, nullptr, 0, true);
    }

    object.Action.Phase = state.action_phase;
    object.Action.Time = ClampActionTicks(state.action_ticks);
    object.Action.Data = state.action_data;
    object.Action.Dir = state.direction;
    object.Action.ComDir = state.command_direction;

    object.UpdatePos();
    object.SetOCF();
}

void SynchronizeRuntimeContainers(
    C4Game &game,
    const LcEngineRuntimeObjectStateSlice &slice) {
    if (!slice.objects || slice.object_count == 0) {
        g_last_container_warning_frame = std::numeric_limits<uint64_t>::max();
        return;
    }

    std::unordered_map<uint64_t, C4Object *> objects;
    objects.reserve(slice.object_count);
    for (size_t index = 0; index < slice.object_count; ++index) {
        const LcEngineRuntimeObjectState &state = slice.objects[index];
        if (state.status == C4OS_DELETED) {
            continue;
        }
        if (C4Object *object = FindObjectByRuntimeId(game, state.id)) {
            objects.emplace(state.id, object);
        }
    }

    std::unordered_set<uint64_t> unresolved_container_ids;
    size_t missing_content_targets = 0;
    size_t reorder_failures = 0;

    for (size_t index = 0; index < slice.object_count; ++index) {
        const LcEngineRuntimeObjectState &state = slice.objects[index];
        if (state.status == C4OS_DELETED) {
            continue;
        }

        auto object_it = objects.find(state.id);
        if (object_it == objects.end()) {
            continue;
        }

        C4Object *object = object_it->second;
        if (!object || !object->Status) {
            continue;
        }

        C4Object *desired_container = nullptr;
        if (state.has_container) {
            auto container_it = objects.find(state.container_id);
            if (container_it != objects.end()) {
                desired_container = container_it->second;
            }
            if (!desired_container || !desired_container->Status ||
                WouldIntroduceContainerCycle(*object, desired_container)) {
                unresolved_container_ids.insert(state.id);
                desired_container = nullptr;
            }
        }

        if (object->Contained == desired_container) {
            if (desired_container && !desired_container->Contents.GetLink(object)) {
                if (desired_container->Contents.Add(object, C4ObjectList::stContents)) {
                    desired_container->UpdateMass();
                    desired_container->SetOCF();
                } else {
                    unresolved_container_ids.insert(state.id);
                }
            }
            object->SetOCF();
            continue;
        }

        if (C4Object *current = object->Contained) {
            if (current->Contents.Remove(object)) {
                current->UpdateMass();
                current->SetOCF();
            }
            if (object->Contained == current) {
                object->Contained = nullptr;
            }
        }

        if (desired_container) {
            bool attached = desired_container->Contents.GetLink(object);
            if (!attached) {
                attached = desired_container->Contents.Add(object, C4ObjectList::stContents);
            }
            if (!attached) {
                unresolved_container_ids.insert(state.id);
                object->Contained = nullptr;
            } else {
                object->Contained = desired_container;
                desired_container->UpdateMass();
                desired_container->SetOCF();
            }
        } else {
            object->Contained = nullptr;
        }

        object->SetOCF();
    }

    for (size_t index = 0; index < slice.object_count; ++index) {
        const LcEngineRuntimeObjectState &state = slice.objects[index];
        if (state.status == C4OS_DELETED || state.contents_len == 0) {
            continue;
        }

        auto container_it = objects.find(state.id);
        if (container_it == objects.end()) {
            continue;
        }

        C4Object *container = container_it->second;
        if (!container || !container->Status) {
            continue;
        }

        std::vector<C4Object *> desired_order;
        desired_order.reserve(state.contents_len);
        std::unordered_set<C4Object *> desired_children;
        desired_children.reserve(state.contents_len * 2);

        for (size_t content_index = 0; content_index < state.contents_len; ++content_index) {
            uint64_t child_id = state.contents[content_index];
            auto child_it = objects.find(child_id);
            if (child_it == objects.end()) {
                ++missing_content_targets;
                continue;
            }

            C4Object *child = child_it->second;
            if (!child || child->Contained != container) {
                ++missing_content_targets;
                continue;
            }

            desired_order.push_back(child);
            desired_children.insert(child);
        }

        bool modified = false;
        for (auto iter = container->Contents.begin(); iter != container->Contents.end();) {
            C4Object *child = *iter;
            ++iter;
            if (!child || child->Contained != container) {
                continue;
            }
            if (desired_children.find(child) == desired_children.end()) {
                if (container->Contents.Remove(child)) {
                    if (child->Contained == container) {
                        child->Contained = nullptr;
                    }
                    child->SetOCF();
                    modified = true;
                }
            }
        }

        if (modified) {
            container->UpdateMass();
            container->SetOCF();
        }

        for (size_t order_index = 1; order_index < desired_order.size(); ++order_index) {
            C4Object *previous = desired_order[order_index - 1];
            C4Object *current = desired_order[order_index];
            if (!previous || !current) {
                continue;
            }
            if (!container->Contents.GetLink(previous) || !container->Contents.GetLink(current)) {
                ++reorder_failures;
                continue;
            }
            if (previous->Status == C4OS_NORMAL && current->Status == C4OS_NORMAL) {
                if (!container->Contents.OrderObjectBefore(previous, current)) {
                    ++reorder_failures;
                }
            }
        }
    }

    if (!unresolved_container_ids.empty() || missing_content_targets > 0 || reorder_failures > 0) {
        if (slice.frame != g_last_container_warning_frame) {
            g_last_container_warning_frame = slice.frame;
            LogWarning(std::format(
                "Rust runtime container sync: {} missing parents, {} missing contents, {} ordering fixes on frame {}",
                unresolved_container_ids.size(),
                missing_content_targets,
                reorder_failures,
                slice.frame));
        }
    } else {
        g_last_container_warning_frame = std::numeric_limits<uint64_t>::max();
    }
}

void ApplyRuntimeObjectStatesToGame(
    C4Game &game,
    const LcEngineRuntimeObjectStateSlice &slice) {
    if (slice.object_count == 0 || slice.objects == nullptr) {
        return;
    }

    std::unordered_map<uint64_t, const LcEngineRuntimeObjectState *> lookup;
    lookup.reserve(slice.object_count);
    for (size_t index = 0; index < slice.object_count; ++index) {
        const LcEngineRuntimeObjectState &state = slice.objects[index];
        lookup.emplace(state.id, &state);
    }

    std::vector<C4Object *> to_remove;
    to_remove.reserve(lookup.size());

    auto reconcile_list = [&](C4ObjectList &list) {
        for (auto it = list.begin(); it != list.end(); ++it) {
            C4Object *object = *it;
            if (!object || !object->Status) {
                continue;
            }
            const uint64_t object_id = static_cast<uint64_t>(object->Number);
            auto found = lookup.find(object_id);
            if (found == lookup.end()) {
                to_remove.push_back(object);
                continue;
            }
            const LcEngineRuntimeObjectState *state = found->second;
            if (state->status == C4OS_DELETED) {
                to_remove.push_back(object);
                lookup.erase(found);
                continue;
            }
            ApplyRuntimeObjectStateToC4Object(*object, *state);
            lookup.erase(found);
        }
    };

    reconcile_list(game.Objects);
    reconcile_list(game.Objects.InactiveObjects);

    for (C4Object *object : to_remove) {
        if (!object) {
            continue;
        }
        object->AssignRemoval();
        game.Objects.Remove(object);
        game.Objects.InactiveObjects.Remove(object);
    }

    if (lookup.empty()) {
        g_last_missing_warning_frame = std::numeric_limits<uint64_t>::max();
        return;
    }

    std::vector<uint64_t> failures;
    failures.reserve(lookup.size());

    for (const auto &[id, state] : lookup) {
        if (state->status == C4OS_DELETED) {
            continue;
        }
        C4Object *object = CreateRuntimeObject(game, *state, slice.frame);
        if (!object) {
            failures.push_back(id);
            continue;
        }
        ApplyRuntimeObjectStateToC4Object(*object, *state);
    }

    SynchronizeRuntimeContainers(game, slice);

    if (!failures.empty()) {
        if (slice.frame != g_last_missing_warning_frame) {
            g_last_missing_warning_frame = slice.frame;
            LogWarning(std::format(
                "Rust runtime could not mirror {} objects on frame {}",
                failures.size(),
                slice.frame));
        }
    } else {
        g_last_missing_warning_frame = std::numeric_limits<uint64_t>::max();
    }
}

void ApplyAuthoritativeEnvironmentState(C4Game &game) {
    if (!g_runtime || g_runtime_disabled) {
        return;
    }

    LcEngineRuntimeEnvironmentState environment{};
    char *error_message = nullptr;
    if (!lc_engine_runtime_export_environment(
            g_runtime.get(),
            &environment,
            &error_message)) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogWarning(std::string("Rust runtime environment export failed: ") + error.get());
        } else {
            LogWarning("Rust runtime environment export failed (no detail)");
        }
        return;
    }

    game.Weather.SetWind(environment.wind);
    game.Weather.SetTemperature(environment.temperature);
    game.Weather.SetClimate(environment.climate);
}

void ApplyAuthoritativeLandscapeState(C4Game &game) {
    if (!g_runtime || g_runtime_disabled) {
        ResetAuthoritativeLandscapeCache();
        return;
    }

    char *error_message = nullptr;
    LcEngineRuntimeLandscapeArray *buffer =
        lc_engine_runtime_export_landscape(g_runtime.get(), &error_message);
    RustStringPtr error = MakeString(error_message);
    if (!buffer) {
        if (error) {
            LogWarning(std::string("Rust runtime landscape export failed: ") + error.get());
        } else {
            LogWarning("Rust runtime landscape export failed (no detail)");
        }
        return;
    }

    const LcEngineRuntimeLandscapeSlice slice =
        lc_engine_runtime_landscape_slice(buffer);

    if (!slice.heights || slice.width == 0) {
        ResetAuthoritativeLandscapeCache();
        lc_engine_runtime_landscape_free(buffer);
        return;
    }

    if (g_authoritative_landscape_heights.size() != slice.width) {
        g_authoritative_landscape_heights.assign(
            slice.width,
            std::numeric_limits<int32_t>::min());
    }

    const int32_t landscape_width = game.Landscape.Width;

    for (uint32_t index = 0; index < slice.width; ++index) {
        const int32_t target_height = slice.heights[index];
        if (index >= static_cast<uint32_t>(landscape_width)) {
            g_authoritative_landscape_heights[index] = target_height;
            continue;
        }

        if (g_authoritative_landscape_heights[index] == target_height) {
            continue;
        }

        const int32_t x = static_cast<int32_t>(index);
        const int32_t current_height = DetermineSurfaceHeight(game, x);
        AdjustLandscapeColumn(game, x, current_height, target_height);
        g_authoritative_landscape_heights[index] = target_height;
    }

    lc_engine_runtime_landscape_free(buffer);
}

void ApplyAuthoritativeRuntimeState(C4Game &game) {
    if (!g_runtime || g_runtime_disabled) {
        return;
    }

    char *error_message = nullptr;
    LcEngineRuntimeObjectStateArray *buffer =
        lc_engine_runtime_export_object_states(g_runtime.get(), &error_message);
    RustStringPtr error = MakeString(error_message);
    if (!buffer) {
        if (error) {
            LogWarning(std::string("Rust runtime state export failed: ") + error.get());
        } else {
            LogWarning("Rust runtime state export failed (no detail)");
        }
        return;
    }

    const LcEngineRuntimeObjectStateSlice slice =
        lc_engine_runtime_object_states_slice(buffer);
    ApplyRuntimeObjectStatesToGame(game, slice);

    lc_engine_runtime_object_states_free(buffer);

    ApplyAuthoritativeEnvironmentState(game);
    ApplyAuthoritativeLandscapeState(game);
}

struct SnapshotEntry {
    LcEngineObjectSnapshot snapshot{};
    std::string definition;
    std::string action;
    std::vector<LcEngineEffectSnapshot> effects;
    std::vector<std::string> effect_names;
    std::vector<LcEngineObjectVertexSnapshot> vertices;
    std::vector<uint64_t> contents;
};

struct CrewSelectionEntry {
    LcEngineCrewSelectionSnapshot snapshot{};
    std::vector<uint64_t> selected;
};

struct CrewRoleEntry {
    LcEngineCrewRoleSnapshot snapshot{};
    std::vector<LcEngineCrewRoleAssignment> assignments;
    std::vector<std::string> role_names;
};

struct HudPlayerEntry {
    LcEngineHudPlayerSnapshot snapshot{};
    std::vector<uint64_t> crew;
};

enum class ParticleLayer : int32_t {
    Global = 0,
    ObjectFront = 1,
    ObjectBack = 2,
};

struct ParticleEntry {
    LcEngineParticleSnapshot snapshot{};
    std::string definition;
};

struct SurfaceEntry {
    LcEngineSurfaceSnapshot snapshot{};
    std::string label;
};

struct SnapshotBuffer {
    std::vector<SnapshotEntry> entries;
    std::vector<LcEngineObjectSnapshot> raw;
    std::vector<LcEngineEffectSnapshot> global_effects;
    std::vector<std::string> global_effect_names;
    std::vector<CrewSelectionEntry> crew_selections;
    std::vector<LcEngineCrewSelectionSnapshot> crew_selection_raw;
    std::vector<CrewRoleEntry> crew_roles;
    std::vector<LcEngineCrewRoleSnapshot> crew_role_raw;
    std::vector<ParticleEntry> particles;
    std::vector<LcEngineParticleSnapshot> particle_raw;
    std::vector<int32_t> known_crew_owners;
    std::vector<int32_t> eliminated_crew_owners;
    std::vector<HudPlayerEntry> hud_players;
    std::vector<LcEngineHudPlayerSnapshot> hud_player_raw;
    std::vector<SurfaceEntry> surfaces;
    std::vector<LcEngineSurfaceSnapshot> surface_raw;
    std::vector<LcEngineNetworkPacketSnapshot> network_packet_raw;
};

std::mutex g_mutex;
uint64_t g_last_missing_warning_frame = std::numeric_limits<uint64_t>::max();
uint64_t g_last_container_warning_frame = std::numeric_limits<uint64_t>::max();
bool g_initialised = false;
bool g_disabled = false;
RecorderPtr g_recorder;
PlaybackPtr g_playback;
std::string g_record_path;
RuntimePtr g_runtime;
bool g_runtime_requested = false;
bool g_runtime_disabled = false;
bool g_runtime_authoritative = false;
std::string g_runtime_state_export_path;
std::string g_runtime_state_import_path;
std::ofstream g_runtime_snapshot_stream;
bool g_runtime_snapshot_enabled = false;
bool g_runtime_snapshot_checked = false;
std::map<uint64_t, std::vector<std::string>> g_frame_controls;
std::mutex g_network_mutex;
std::map<uint64_t, std::vector<LcEngineNetworkPacketSnapshot>> g_frame_network_packets;
std::atomic<bool> g_capture_network_packets{false};
std::vector<int32_t> g_authoritative_landscape_heights;

void ResetAuthoritativeLandscapeCache() {
    g_authoritative_landscape_heights.clear();
}

int32_t DetermineSurfaceHeight(C4Game &game, int32_t x) {
    const int32_t width = game.Landscape.Width;
    const int32_t height = game.Landscape.Height;
    if (x < 0 || x >= width) {
        return height;
    }

    for (int32_t y = 0; y < height; ++y) {
        if (game.Landscape.GetDensity(x, y) >= C4M_Solid) {
            return y;
        }
    }

    return height;
}

bool AdjustLandscapeColumn(C4Game &game, int32_t x, int32_t current_height, int32_t target_height) {
    const int32_t width = game.Landscape.Width;
    const int32_t height = game.Landscape.Height;
    if (x < 0 || x >= width) {
        return false;
    }

    int32_t clamped_target = std::clamp(target_height, 0, height);
    if (clamped_target == current_height) {
        return false;
    }

    if (clamped_target > current_height) {
        const int32_t clamped_end = std::min(clamped_target, height);
        if (clamped_end > current_height) {
            game.Landscape.ClearRect(x, current_height, 1, clamped_end - current_height);
            return true;
        }
        return false;
    }

    if (current_height > height) {
        current_height = height;
    }

    const int32_t fill_start = std::max(clamped_target, 0);
    const int32_t fill_end = std::min(current_height, height);
    if (fill_start >= fill_end) {
        return false;
    }

    int32_t sample_y = fill_end;
    if (sample_y >= height) {
        sample_y = height - 1;
    }
    if (sample_y < 0) {
        return false;
    }

    const uint8_t pix = game.Landscape.GetPix(x, sample_y);
    C4Rect rect(x, fill_start, 1, fill_end - fill_start);
    game.Landscape.PrepareChange(rect, false);
    for (int32_t y = fill_start; y < fill_end; ++y) {
        game.Landscape.SetPix(x, y, pix);
    }
    game.Landscape.FinishChange(rect, false);
    return true;
}

RustStringPtr MakeString(char *raw) {
    return RustStringPtr(raw, lc_engine_string_free);
}

std::string SerialiseControl(const C4Control &control) {
    if (!control.firstPkt()) {
        return {};
    }

    C4Control copy;
    copy.Copy(control);
    try {
        return DecompileToBuf<StdCompilerINIWrite>(mkNamingAdapt(copy, "Control"));
    } catch (const std::exception &exception) {
        LogWarning(std::string("Failed to serialise control for Rust runtime: ") + exception.what());
    } catch (...) {
        LogWarning("Failed to serialise control for Rust runtime (unknown error)");
    }
    return {};
}

void LogWarning(const std::string &message) {
    LogNTr(spdlog::level::warn, message);
}

void LogError(const std::string &message) {
    LogNTr(spdlog::level::err, message);
}

void ClearNetworkPacketLog() {
    std::lock_guard<std::mutex> lock(g_network_mutex);
    g_frame_network_packets.clear();
}

std::string LoadFile(const std::string &path) {
    std::ifstream stream(path);
    if (!stream) {
        return {};
    }
    return std::string(std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>());
}

bool LoadFileIntoString(const std::string &path, std::string &output) {
    std::ifstream stream(path);
    if (!stream) {
        return false;
    }
    output.assign(std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>());
    return true;
}

bool ImportRuntimeState(RuntimePtr &runtime, const std::string &path) {
    if (!runtime || path.empty()) {
        return true;
    }

    std::string json;
    if (!LoadFileIntoString(path, json)) {
        LogWarning(std::string("Rust runtime import state path could not be opened (runtime parity disabled): ") + path);
        return false;
    }

    const bool has_content = std::any_of(
        json.begin(),
        json.end(),
        [](unsigned char ch) { return !std::isspace(ch); });
    if (!has_content) {
        LogWarning(std::string("Rust runtime import state path contained no data (runtime parity disabled): ") + path);
        return false;
    }

    char *error_message = nullptr;
    if (!lc_engine_runtime_import_state_json(runtime.get(), json.c_str(), &error_message)) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogWarning(std::string("Rust runtime failed to import state (runtime parity disabled): ") + error.get());
        } else {
            LogWarning("Rust runtime failed to import state (runtime parity disabled, no detail)");
        }
        return false;
    }

    return true;
}

std::string DetermineScenarioPath(const C4Game &game) {
    const StdStrBuf full_name = game.ScenarioFile.GetFullName();
    if (full_name.getLength()) {
        return std::string(full_name.getData());
    }
    if (game.ScenarioFilename[0]) {
        return std::string(game.ScenarioFilename);
    }
    return {};
}

bool InitialiseRuntime(C4Game &game) {
    if (!g_runtime_requested || g_runtime_disabled) {
        return false;
    }

    RuntimePtr runtime(lc_engine_runtime_new());
    if (!runtime) {
        LogWarning("Rust runtime could not be created");
        g_runtime_disabled = true;
        ResetAuthoritativeLandscapeCache();
        return false;
    }

    const std::string scenario_path = DetermineScenarioPath(game);
    if (scenario_path.empty()) {
        LogWarning("Rust runtime could not determine scenario path");
        g_runtime_disabled = true;
        ResetAuthoritativeLandscapeCache();
        return false;
    }

    char *error_message = nullptr;
    const uint64_t seed =
        static_cast<uint64_t>(static_cast<uint32_t>(game.Parameters.RandomSeed));
    // The Rust runtime synthesizes the static-map landscape itself
    // (ChunkOZoom); its chunk jitter must use the SAME MapSeed the C++
    // landscape drew at init (C4Landscape.cpp:563), handed across here
    // because the Rust RNG is not the C++ LCG yet.
    setenv("LC_RUST_ENGINE_MAP_SEED", std::to_string(game.Landscape.MapSeed).c_str(), 1);
    if (!lc_engine_runtime_load_scenario(runtime.get(), scenario_path.c_str(), seed, &error_message)) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogError(std::string("Rust runtime failed to load scenario: ") + error.get());
        } else {
            LogError("Rust runtime failed to load scenario (no detail)");
        }
        g_runtime_disabled = true;
        ResetAuthoritativeLandscapeCache();
        return false;
    }

    if (!ImportRuntimeState(runtime, g_runtime_state_import_path)) {
        g_runtime_disabled = true;
        ResetAuthoritativeLandscapeCache();
        return false;
    }

    g_runtime = std::move(runtime);
    g_runtime_disabled = false;
    ResetAuthoritativeLandscapeCache();
    return true;
}

void CollectParticlesFromList(
    C4ParticleList &list,
    ParticleLayer layer,
    uint64_t owner_id,
    SnapshotBuffer &buffer) {
    for (C4Particle *particle = list.First(); particle; particle = C4ParticleList::Next(particle)) {
        if (!particle || !particle->pDef) {
            continue;
        }
        ParticleEntry entry;
        if (const char *name = particle->pDef->Name.getData()) {
            entry.definition = name;
        } else {
            entry.definition.clear();
        }
        entry.snapshot.x = particle->x;
        entry.snapshot.y = particle->y;
        entry.snapshot.xdir = particle->xdir;
        entry.snapshot.ydir = particle->ydir;
        entry.snapshot.life = particle->life;
        entry.snapshot.parameter_a = particle->a;
        entry.snapshot.parameter_b = particle->b;
        entry.snapshot.layer = static_cast<int32_t>(layer);
        entry.snapshot.has_owner = owner_id != 0;
        entry.snapshot.owner_id = owner_id;
        buffer.particles.push_back(std::move(entry));
    }
}

SnapshotBuffer CollectSnapshotBuffer(C4Game &game, bool capture_surface_hash) {
    SnapshotBuffer buffer;
    const uint64_t frame = static_cast<uint64_t>(game.FrameCounter);
    CollectParticlesFromList(
        game.Particles.GlobalParticles,
        ParticleLayer::Global,
        0,
        buffer);
    std::set<int32_t> active_owners;
    for (auto it = game.Objects.begin(); it != game.Objects.end(); ++it) {
        C4Object *object = *it;
        if (!object || !object->Status) {
            continue;
        }
        const uint64_t object_id = static_cast<uint64_t>(object->Number);
        CollectParticlesFromList(object->FrontParticles, ParticleLayer::ObjectFront, object_id, buffer);
        CollectParticlesFromList(object->BackParticles, ParticleLayer::ObjectBack, object_id, buffer);
        SnapshotEntry entry;
        entry.snapshot.id = static_cast<uint64_t>(object->Number);
        // The runtime keys objects by C4ID text; the display name would make
        // every per-definition comparison miss.
        entry.definition = object->Def ? C4IdText(object->Def->id) : "";
        entry.snapshot.position_x = fixtoi(object->fix_x);
        entry.snapshot.position_y = fixtoi(object->fix_y);
        entry.snapshot.velocity_x = fixtoi(object->xdir);
        entry.snapshot.velocity_y = fixtoi(object->ydir);
        entry.snapshot.rotation = object->r;
        entry.snapshot.fixed_position_x = object->fix_x.val;
        entry.snapshot.fixed_position_y = object->fix_y.val;
        entry.snapshot.fixed_velocity_x = object->xdir.val;
        entry.snapshot.fixed_velocity_y = object->ydir.val;
        entry.snapshot.fixed_rotation = object->fix_r.val;
        entry.snapshot.rotation_velocity = object->rdir.val;
        entry.snapshot.energy = object->Energy;
        entry.snapshot.damage = object->Damage;
        entry.snapshot.magic_energy = object->MagicEnergy;
        if (const auto *physical = object->GetPhysical()) {
            entry.snapshot.magic_capacity = physical->Magic;
        } else {
            entry.snapshot.magic_capacity = 0;
        }
        entry.snapshot.owner = static_cast<int32_t>(object->Owner);
        entry.snapshot.category = object->Category;
        entry.snapshot.crew_member = (object->OCF & OCF_CrewMember) != 0;
        entry.snapshot.alive = object->GetAlive();
        entry.action = object->Action.Name;
        entry.snapshot.action_name = entry.action.c_str();
        entry.snapshot.action_phase = object->Action.Phase;
        entry.snapshot.action_ticks = object->Action.Time;
        entry.snapshot.action_data = object->Action.Data;
        entry.snapshot.direction = object->Action.Dir;
        entry.snapshot.command_direction = object->Action.ComDir;

        if (entry.snapshot.crew_member && entry.snapshot.owner != NO_OWNER) {
            active_owners.insert(entry.snapshot.owner);
        }

        if (object->Contained) {
            if (C4Object *container = object->Contained.Object()) {
                if (container->Status) {
                    entry.snapshot.has_container = true;
                    entry.snapshot.container_id =
                        static_cast<uint64_t>(container->Number);
                }
            }
        }

        for (C4ObjectLink *link = object->Contents.First; link; link = link->Next) {
            C4Object *contained = link->Obj;
            if (!contained || !contained->Status) {
                continue;
            }
            entry.contents.push_back(static_cast<uint64_t>(contained->Number));
        }

        if (object->Shape.VtxNum > 0) {
            entry.vertices.reserve(object->Shape.VtxNum);
            for (int32_t vertex_index = 0; vertex_index < object->Shape.VtxNum; ++vertex_index) {
                LcEngineObjectVertexSnapshot vertex{};
                vertex.x = object->Shape.VtxX[vertex_index];
                vertex.y = object->Shape.VtxY[vertex_index];
                vertex.cnat = static_cast<uint32_t>(object->Shape.VtxCNAT[vertex_index]);
                vertex.friction = object->Shape.VtxFriction[vertex_index];
                entry.vertices.push_back(vertex);
            }
        }

        if (object->pEffects) {
            size_t effect_count = 0;
            for (C4Effect *effect = object->pEffects; effect; effect = effect->pNext) {
                if (effect->IsDead()) {
                    continue;
                }
                ++effect_count;
            }

            if (effect_count > 0) {
                entry.effect_names.reserve(effect_count);
                entry.effects.resize(effect_count);

                size_t index = 0;
                for (C4Effect *effect = object->pEffects; effect; effect = effect->pNext) {
                    if (effect->IsDead()) {
                        continue;
                    }

                    entry.effect_names.emplace_back(effect->Name);

                    LcEngineEffectSnapshot effect_snapshot{};
                    effect_snapshot.priority = effect->iPriority;
                    effect_snapshot.interval = effect->iIntervall;
                    effect_snapshot.timer = effect->iTime;
                    entry.effects[index] = effect_snapshot;
                    ++index;
                }

                for (size_t i = 0; i < entry.effects.size(); ++i) {
                    entry.effects[i].name = entry.effect_names[i].c_str();
                }
            }
        }
        buffer.entries.push_back(std::move(entry));
    }

    std::sort(buffer.entries.begin(), buffer.entries.end(), [](const SnapshotEntry &lhs, const SnapshotEntry &rhs) {
        return lhs.snapshot.id < rhs.snapshot.id;
    });

    buffer.raw.reserve(buffer.entries.size());
    for (auto &entry : buffer.entries) {
        entry.snapshot.definition_id = entry.definition.c_str();
        entry.snapshot.action_name = entry.action.c_str();
        entry.snapshot.effects = entry.effects.empty() ? nullptr : entry.effects.data();
        entry.snapshot.effect_count = entry.effects.size();
        entry.snapshot.vertices = entry.vertices.empty() ? nullptr : entry.vertices.data();
        entry.snapshot.vertex_count = entry.vertices.size();
        entry.snapshot.contents = entry.contents.empty() ? nullptr : entry.contents.data();
        entry.snapshot.contents_count = entry.contents.size();
        buffer.raw.push_back(entry.snapshot);
    }

    std::sort(
        buffer.particles.begin(),
        buffer.particles.end(),
        [](const ParticleEntry &lhs, const ParticleEntry &rhs) {
            if (lhs.snapshot.layer != rhs.snapshot.layer) {
                return lhs.snapshot.layer < rhs.snapshot.layer;
            }
            if (lhs.snapshot.owner_id != rhs.snapshot.owner_id) {
                return lhs.snapshot.owner_id < rhs.snapshot.owner_id;
            }
            int comparison = lhs.definition.compare(rhs.definition);
            if (comparison != 0) {
                return comparison < 0;
            }
            if (lhs.snapshot.x != rhs.snapshot.x) {
                return lhs.snapshot.x < rhs.snapshot.x;
            }
            if (lhs.snapshot.y != rhs.snapshot.y) {
                return lhs.snapshot.y < rhs.snapshot.y;
            }
            if (lhs.snapshot.xdir != rhs.snapshot.xdir) {
                return lhs.snapshot.xdir < rhs.snapshot.xdir;
            }
            if (lhs.snapshot.ydir != rhs.snapshot.ydir) {
                return lhs.snapshot.ydir < rhs.snapshot.ydir;
            }
            if (lhs.snapshot.life != rhs.snapshot.life) {
                return lhs.snapshot.life < rhs.snapshot.life;
            }
            if (lhs.snapshot.parameter_a != rhs.snapshot.parameter_a) {
                return lhs.snapshot.parameter_a < rhs.snapshot.parameter_a;
            }
            return lhs.snapshot.parameter_b < rhs.snapshot.parameter_b;
        });

    buffer.particle_raw.reserve(buffer.particles.size());
    for (auto &entry : buffer.particles) {
        entry.snapshot.definition_id = entry.definition.c_str();
        buffer.particle_raw.push_back(entry.snapshot);
    }

    for (C4Effect *effect = game.pGlobalEffects; effect; effect = effect->pNext) {
        if (effect->IsDead()) {
            continue;
        }

        buffer.global_effect_names.emplace_back(effect->Name);
        LcEngineEffectSnapshot effect_snapshot{};
        effect_snapshot.priority = effect->iPriority;
        effect_snapshot.interval = effect->iIntervall;
        effect_snapshot.timer = effect->iTime;
        buffer.global_effects.push_back(effect_snapshot);
    }

    for (size_t i = 0; i < buffer.global_effects.size(); ++i) {
        buffer.global_effects[i].name = buffer.global_effect_names[i].c_str();
    }

    for (C4Player *player = game.Players.First; player; player = player->Next) {
        if (!player) {
            continue;
        }
        const int32_t owner = static_cast<int32_t>(player->Number);
        if (owner == NO_OWNER) {
            continue;
        }

        CrewSelectionEntry selection_entry;
        selection_entry.snapshot.owner = owner;
        if (player->Cursor && player->Cursor->Status) {
            selection_entry.snapshot.has_cursor = true;
            selection_entry.snapshot.cursor = static_cast<uint64_t>(player->Cursor->Number);
        }

        for (C4ObjectLink *link = player->Crew.First; link; link = link->Next) {
            C4Object *crew = link->Obj;
            if (!crew || !crew->Status) {
                continue;
            }
            if (crew->Select) {
                selection_entry.selected.push_back(static_cast<uint64_t>(crew->Number));
            }
        }

        if (!selection_entry.selected.empty() || selection_entry.snapshot.has_cursor) {
            buffer.crew_selections.push_back(std::move(selection_entry));
        }

        HudPlayerEntry hud_entry;
        hud_entry.snapshot.owner = owner;
        hud_entry.snapshot.eliminated = player->Eliminated != 0;
        hud_entry.snapshot.wealth = static_cast<int32_t>(player->Wealth);
        hud_entry.snapshot.score = static_cast<int32_t>(player->Score);
        if (player->Cursor && player->Cursor->Status) {
            hud_entry.snapshot.has_focus = true;
            hud_entry.snapshot.focus_object = static_cast<uint64_t>(player->Cursor->Number);
        } else if (player->ViewTarget && player->ViewTarget->Status) {
            hud_entry.snapshot.has_focus = true;
            hud_entry.snapshot.focus_object = static_cast<uint64_t>(player->ViewTarget->Number);
        } else {
            hud_entry.snapshot.has_focus = false;
            hud_entry.snapshot.focus_object = 0;
        }

        for (C4ObjectLink *link = player->Crew.First; link; link = link->Next) {
            C4Object *crew = link->Obj;
            if (!crew || !crew->Status) {
                continue;
            }
            hud_entry.crew.push_back(static_cast<uint64_t>(crew->Number));
        }
        std::sort(hud_entry.crew.begin(), hud_entry.crew.end());
        hud_entry.snapshot.crew = hud_entry.crew.empty() ? nullptr : hud_entry.crew.data();
        hud_entry.snapshot.crew_count = hud_entry.crew.size();
        buffer.hud_players.push_back(std::move(hud_entry));

        if (player->Eliminated) {
            buffer.eliminated_crew_owners.push_back(owner);
        }
    }

    buffer.crew_selection_raw.reserve(buffer.crew_selections.size());
    for (auto &entry : buffer.crew_selections) {
        entry.snapshot.selected = entry.selected.empty() ? nullptr : entry.selected.data();
        entry.snapshot.selected_count = entry.selected.size();
        buffer.crew_selection_raw.push_back(entry.snapshot);
        active_owners.insert(entry.snapshot.owner);
    }

    buffer.crew_role_raw.reserve(buffer.crew_roles.size());
    for (auto &entry : buffer.crew_roles) {
        for (size_t i = 0; i < entry.assignments.size(); ++i) {
            entry.assignments[i].role = entry.role_names[i].c_str();
        }
        entry.snapshot.assignments =
            entry.assignments.empty() ? nullptr : entry.assignments.data();
        entry.snapshot.assignment_count = entry.assignments.size();
        buffer.crew_role_raw.push_back(entry.snapshot);
        active_owners.insert(entry.snapshot.owner);
    }

    buffer.hud_player_raw.reserve(buffer.hud_players.size());
    for (auto &entry : buffer.hud_players) {
        buffer.hud_player_raw.push_back(entry.snapshot);
        active_owners.insert(entry.snapshot.owner);
    }

    std::sort(
        buffer.eliminated_crew_owners.begin(),
        buffer.eliminated_crew_owners.end());
    buffer.eliminated_crew_owners.erase(
        std::unique(
            buffer.eliminated_crew_owners.begin(),
            buffer.eliminated_crew_owners.end()),
        buffer.eliminated_crew_owners.end());

    std::set<int32_t> known_owners = active_owners;
    for (int32_t owner : buffer.eliminated_crew_owners) {
        known_owners.insert(owner);
    }
    buffer.known_crew_owners.assign(known_owners.begin(), known_owners.end());
    std::sort(buffer.known_crew_owners.begin(), buffer.known_crew_owners.end());

    if (capture_surface_hash) {
        CStdDDraw *ddraw = Application.DDraw;
        if (ddraw && ddraw->lpBack) {
            C4Surface *surface = ddraw->lpBack;
            SurfaceLock lock(surface);
            if (lock.IsLocked()) {
                const float scale = Application.GetScale();
                const int32_t surface_width =
                    static_cast<int32_t>(std::ceil(surface->Wdt * scale));
                const int32_t surface_height =
                    static_cast<int32_t>(std::ceil(surface->Hgt * scale));
                if (surface_width > 0 && surface_height > 0) {
                    const auto clamp_coordinate = [](int32_t value, int32_t maximum) -> int32_t {
                        if (value < 0) {
                            return 0;
                        }
                        if (value > maximum) {
                            return maximum;
                        }
                        return value;
                    };
                    auto add_surface_hash = [&](std::string label,
                                                int32_t origin_x,
                                                int32_t origin_y,
                                                int32_t width,
                                                int32_t height) {
                        if (width <= 0 || height <= 0) {
                            return;
                        }
                        origin_x = clamp_coordinate(origin_x, surface_width);
                        origin_y = clamp_coordinate(origin_y, surface_height);
                        width = std::min(width, surface_width - origin_x);
                        height = std::min(height, surface_height - origin_y);
                        if (width <= 0 || height <= 0) {
                            return;
                        }
                        SurfaceEntry entry;
                        entry.label = std::move(label);
                        entry.snapshot.width = width;
                        entry.snapshot.height = height;
                        entry.snapshot.hash = HashSurfaceRegion(
                            *surface,
                            origin_x,
                            origin_y,
                            width,
                            height,
                            scale);
                        buffer.surfaces.push_back(std::move(entry));
                    };
                    auto add_surface_from_rect =
                        [&](std::string label, int32_t x, int32_t y, int32_t width, int32_t height) {
                            if (width <= 0 || height <= 0) {
                                return;
                            }
                            const double left_value =
                                static_cast<double>(x) * static_cast<double>(scale);
                            const double top_value =
                                static_cast<double>(y) * static_cast<double>(scale);
                            const double right_value =
                                static_cast<double>(x + width) * static_cast<double>(scale);
                            const double bottom_value =
                                static_cast<double>(y + height) * static_cast<double>(scale);
                            int32_t left = static_cast<int32_t>(std::floor(left_value));
                            int32_t top = static_cast<int32_t>(std::floor(top_value));
                            int32_t right = static_cast<int32_t>(std::ceil(right_value));
                            int32_t bottom = static_cast<int32_t>(std::ceil(bottom_value));
                            left = clamp_coordinate(left, surface_width);
                            top = clamp_coordinate(top, surface_height);
                            right = clamp_coordinate(right, surface_width);
                            bottom = clamp_coordinate(bottom, surface_height);
                            const int32_t scaled_width = right - left;
                            const int32_t scaled_height = bottom - top;
                            add_surface_hash(
                                std::move(label),
                                left,
                                top,
                                scaled_width,
                                scaled_height);
                        };

                    add_surface_hash(
                        "back_buffer",
                        0,
                        0,
                        surface_width,
                        surface_height);

                    const auto &viewports = Game.GraphicsSystem.GetViewports();
                    size_t viewport_index = 0;
                    for (const auto &viewport_ptr : viewports) {
                        if (!viewport_ptr) {
                            continue;
                        }
                        C4Viewport *viewport =
                            const_cast<C4Viewport *>(viewport_ptr.get());
                        const C4Rect rect = viewport->GetOutputRect();
                        std::string label = "viewport#" + std::to_string(viewport_index);
                        label += ":player=";
                        const int32_t player = viewport->GetPlayer();
                        if (player == NO_OWNER) {
                            label += "none";
                        } else {
                            label += std::to_string(player);
                        }
                        add_surface_from_rect(label, rect.x, rect.y, rect.Wdt, rect.Hgt);
                        ++viewport_index;
                    }

                    const C4Facet &upper_output = Game.GraphicsSystem.UpperBoard.GetOutputFacet();
                    if (upper_output.Surface == surface &&
                        upper_output.Wdt > 0 &&
                        upper_output.Hgt > 0) {
                        add_surface_from_rect(
                            "upper_board",
                            upper_output.X,
                            upper_output.Y,
                            upper_output.Wdt,
                            upper_output.Hgt);
                    }

                    const C4Facet &message_output = Game.GraphicsSystem.MessageBoard.Output;
                    if (message_output.Surface == surface &&
                        message_output.Wdt > 0 &&
                        message_output.Hgt > 0) {
                        add_surface_from_rect(
                            "message_board",
                            message_output.X,
                            message_output.Y,
                            message_output.Wdt,
                            message_output.Hgt);
                    }
                }
            }
        }
    }

    buffer.surface_raw.reserve(buffer.surfaces.size());
    for (auto &entry : buffer.surfaces) {
        entry.snapshot.label = entry.label.c_str();
        buffer.surface_raw.push_back(entry.snapshot);
    }

    {
        std::lock_guard<std::mutex> network_lock(g_network_mutex);
        auto it = g_frame_network_packets.find(frame);
        if (it != g_frame_network_packets.end()) {
            buffer.network_packet_raw = std::move(it->second);
            g_frame_network_packets.erase(it);
        }
    }

    return buffer;
}

void ResetRecorder() {
    g_recorder.reset();
    g_record_path.clear();
}

bool InitialiseRecorder(const char *path) {
    if (!path || !*path) {
        return false;
    }
    RecorderPtr recorder(lc_engine_recorder_new());
    if (!recorder) {
        LogWarning("Rust engine recorder could not be created");
        return false;
    }
    g_recorder = std::move(recorder);
    g_record_path = path;
    return true;
}

bool InitialisePlayback(const std::string &json) {
    if (json.empty()) {
        LogWarning("Rust engine playback baseline is empty");
        return false;
    }
    char *error_message = nullptr;
    PlaybackPtr playback(lc_engine_playback_from_json(json.c_str(), &error_message));
    if (!playback) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogError(std::string("Failed to load Rust playback baseline: ") + error.get());
        } else {
            LogError("Failed to load Rust playback baseline");
        }
        return false;
    }
    g_playback = std::move(playback);
    return true;
}

void EnsureInitialised() {
    if (g_initialised) {
        return;
    }
    g_initialised = true;

    g_runtime_requested = false;
    g_runtime_disabled = false;
    g_runtime_authoritative = false;
    g_runtime_state_export_path.clear();
    g_runtime_state_import_path.clear();

    const char *record_path = std::getenv("LC_RUST_ENGINE_RECORD");
    if (record_path && *record_path) {
        InitialiseRecorder(record_path);
    }

    if (const char *runtime_toggle = std::getenv("LC_RUST_ENGINE_RUNTIME")) {
        if (*runtime_toggle) {
            g_runtime_requested = true;
        }
    }

    if (const char *authoritative =
            std::getenv("LC_RUST_ENGINE_RUNTIME_AUTHORITATIVE")) {
        if (authoritative && *authoritative) {
            g_runtime_requested = true;
            g_runtime_authoritative = true;
        }
    }

    if (const char *state_path = std::getenv("LC_RUST_ENGINE_RUNTIME_STATE")) {
        if (state_path[0]) {
            g_runtime_state_export_path = state_path;
        }
    }

    if (const char *import_path = std::getenv("LC_RUST_ENGINE_RUNTIME_STATE_IMPORT")) {
        if (import_path[0]) {
            g_runtime_state_import_path = import_path;
        }
    }

    if (!g_runtime_snapshot_checked) {
        g_runtime_snapshot_checked = true;
        if (const char *snapshot_path = std::getenv("LC_RUST_ENGINE_RUNTIME_SNAPSHOT")) {
            if (*snapshot_path) {
                g_runtime_snapshot_stream.close();
                g_runtime_snapshot_stream.open(snapshot_path, std::ios::out | std::ios::trunc);
                if (!g_runtime_snapshot_stream) {
                    LogWarning(std::string("Failed to open Rust runtime snapshot path: ") + snapshot_path);
                } else {
                    g_runtime_snapshot_enabled = true;
                }
            }
        }
    }

    if (const char *playback_path = std::getenv("LC_RUST_ENGINE_PLAYBACK")) {
        if (*playback_path) {
            std::string json = LoadFile(playback_path);
            if (!InitialisePlayback(json)) {
                g_disabled = true;
            }
        }
    }

    if (!g_recorder && !g_playback && !g_runtime_requested) {
        g_disabled = true;
    }
}

void FinishPlayback() {
    if (!g_playback) {
        return;
    }
    char *error_message = nullptr;
    if (!lc_engine_playback_finish(g_playback.release(), &error_message)) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogWarning(std::string("Rust playback validation did not finish cleanly: ") + error.get());
        } else {
            LogWarning("Rust playback validation did not finish cleanly");
        }
    }
}

void FlushRecording() {
    if (!g_recorder) {
        return;
    }
    if (g_record_path.empty()) {
        ResetRecorder();
        return;
    }

    char *json_ptr = lc_engine_recorder_export_json(g_recorder.get());
    RustStringPtr json = MakeString(json_ptr);
    if (!json) {
        LogWarning("Rust engine recorder could not export data");
        ResetRecorder();
        return;
    }

    std::ofstream out(g_record_path);
    if (!out) {
        LogWarning(std::string("Failed to open Rust engine recording path: ") + g_record_path);
        ResetRecorder();
        return;
    }

    out << json.get();
    if (!out.good()) {
        LogWarning(std::string("Failed to write Rust engine recording to path: ") + g_record_path);
    }

    ResetRecorder();
}

void ExportRuntimeState() {
    if (!g_runtime || g_runtime_state_export_path.empty()) {
        return;
    }

    char *error_message = nullptr;
    char *json_ptr = lc_engine_runtime_export_state_json(g_runtime.get(), &error_message);
    RustStringPtr error = MakeString(error_message);
    RustStringPtr json = MakeString(json_ptr);
    if (!json) {
        if (error) {
            LogWarning(std::string("Failed to capture Rust runtime state: ") + error.get());
        } else {
            LogWarning("Failed to capture Rust runtime state (no detail)");
        }
        return;
    }

    std::ofstream out(g_runtime_state_export_path, std::ios::out | std::ios::trunc);
    if (!out) {
        LogWarning(std::string("Failed to open Rust runtime state path: ") + g_runtime_state_export_path);
        return;
    }

    out << json.get();
    if (!out.good()) {
        LogWarning(std::string("Failed to write Rust runtime state to path: ") + g_runtime_state_export_path);
    }
}

} // namespace

namespace RustEngineBridge {

bool IsActive() {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    const bool runtime_active = g_runtime_requested && !g_runtime_disabled;
    return (!g_disabled && (g_recorder || g_playback)) || runtime_active;
}

bool FindPath(
    C4Game &game,
    int32_t from_x,
    int32_t from_y,
    int32_t to_x,
    int32_t to_y,
    bool transfer_zones_enabled,
    int32_t level,
    bool (*set_waypoint)(int32_t, int32_t, intptr_t, intptr_t),
    intptr_t parameter) {
    if (!set_waypoint) {
        return false;
    }

    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    if (!g_runtime || g_runtime_disabled) {
        return false;
    }

    char *error_message = nullptr;
    LcEngineRuntimePathResult *buffer = lc_engine_runtime_find_path(
        g_runtime.get(),
        from_x,
        from_y,
        to_x,
        to_y,
        transfer_zones_enabled,
        level,
        &error_message);
    if (!buffer) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogWarning(std::string("Rust pathfinder request failed: ") + error.get());
        } else {
            LogWarning("Rust pathfinder request failed (no detail)");
        }
        return false;
    }

    LcEnginePathSlice slice = lc_engine_runtime_path_slice(buffer);
    if (!slice.found || slice.waypoint_count == 0 || slice.waypoints == nullptr) {
        lc_engine_runtime_path_free(buffer);
        return false;
    }

    size_t start_index = 0;
    const LcEnginePathWaypoint *waypoints = slice.waypoints;
    if (waypoints && slice.waypoint_count > 0) {
        const LcEnginePathWaypoint &first = waypoints[0];
        if (first.x == from_x && first.y == from_y) {
            start_index = 1;
        }
    }

    bool success = true;
    for (size_t index = start_index; index < slice.waypoint_count; ++index) {
        const LcEnginePathWaypoint &waypoint = waypoints[index];
        intptr_t transfer_target_ptr = 0;
        if (waypoint.has_transfer_target) {
            if (C4Object *target =
                    FindObjectByRuntimeId(game, waypoint.transfer_target)) {
                transfer_target_ptr = reinterpret_cast<intptr_t>(target);
            }
        }
        if (!set_waypoint(waypoint.x, waypoint.y, transfer_target_ptr, parameter)) {
            success = false;
            break;
        }
    }

    lc_engine_runtime_path_free(buffer);
    return success;
}

void OnGameStart(C4Game &game) {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    g_frame_controls.clear();
    ClearNetworkPacketLog();
    g_capture_network_packets.store(false, std::memory_order_release);
    if (!g_runtime_requested) {
        if (!g_disabled && (g_recorder || g_playback)) {
            g_capture_network_packets.store(true, std::memory_order_release);
        }
        return;
    }

    g_runtime.reset();
    g_runtime_disabled = false;
    if (!InitialiseRuntime(game)) {
        g_runtime.reset();
    }

    if (!g_disabled && (g_recorder || g_playback || (g_runtime && !g_runtime_disabled))) {
        g_capture_network_packets.store(true, std::memory_order_release);
    }
}

void OnControlFrame(const C4Control &control, uint64_t frame) {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    std::string serialised = SerialiseControl(control);
    if (serialised.empty()) {
        return;
    }

    auto &control_log = g_frame_controls[frame];
    control_log.push_back(std::move(serialised));
    const std::string &stored_control = control_log.back();

    // Env-gated diagnostics: dump every serialised control frame so the
    // Rust-side INI parser can be checked against the real wire format.
    if (const char *dump_path = std::getenv("LC_RUST_ENGINE_CONTROL_DUMP"); dump_path && *dump_path) {
        std::ofstream dump{dump_path, std::ios::app};
        if (dump) {
            dump << "=== frame " << frame << " ===\n" << stored_control << "\n";
        }
    }

    if (!g_runtime_requested || g_runtime_disabled || !g_runtime) {
        return;
    }

    char *error_message = nullptr;
    if (!lc_engine_runtime_record_control_ini(
            g_runtime.get(),
            frame,
            stored_control.c_str(),
            &error_message)) {
        RustStringPtr error = MakeString(error_message);
        if (error) {
            LogWarning(std::string("Rust runtime control capture failed: ") + error.get());
        } else {
            LogWarning("Rust runtime control capture failed (no detail)");
        }
    }
}

void OnNetworkPacket(
    uint8_t status,
    const uint8_t *payload,
    size_t payload_size,
    int32_t client_id,
    uint32_t connection_id,
    bool inbound) {
    if (!g_capture_network_packets.load(std::memory_order_acquire)) {
        return;
    }

    const size_t bounded_size = std::min(payload_size, static_cast<size_t>(std::numeric_limits<uint32_t>::max()));
    const uint8_t *data = payload;
    uint64_t hash = HashNetworkPacket(status, data, bounded_size);

    LcEngineNetworkPacketSnapshot snapshot{};
    snapshot.direction = inbound ? 0u : 1u;
    snapshot.status = status;
    snapshot.size = static_cast<uint32_t>(bounded_size);
    snapshot.hash = hash;
    snapshot.client_id = client_id;
    snapshot.connection_id = connection_id;

    uint64_t frame = 0;
    if (Game.FrameCounter >= 0) {
        frame = static_cast<uint64_t>(Game.FrameCounter);
    }

    {
        std::lock_guard<std::mutex> lock(g_network_mutex);
        g_frame_network_packets[frame].push_back(snapshot);
    }
}

void OnFrame(C4Game &game) {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    if (g_disabled) {
        g_frame_controls.clear();
        ClearNetworkPacketLog();
        g_capture_network_packets.store(false, std::memory_order_release);
        return;
    }

    g_capture_network_packets.store(true, std::memory_order_release);

    const bool capture_surface_hash =
        (g_recorder != nullptr) ||
        (g_playback != nullptr) ||
        g_runtime_snapshot_enabled ||
        (g_runtime != nullptr && !g_runtime_disabled);
    SnapshotBuffer buffer = CollectSnapshotBuffer(game, capture_surface_hash);
    const auto &raw = buffer.raw;
    const auto &global_effects = buffer.global_effects;
    const auto &crew_selection = buffer.crew_selection_raw;
    const auto &crew_roles = buffer.crew_role_raw;
    const auto &particles = buffer.particle_raw;
    const auto &hud_players = buffer.hud_player_raw;
    const LcEngineObjectSnapshot *object_data = raw.empty() ? nullptr : raw.data();
    const LcEngineEffectSnapshot *global_effect_data =
        global_effects.empty() ? nullptr : global_effects.data();
    const LcEngineParticleSnapshot *particle_data =
        particles.empty() ? nullptr : particles.data();
    const LcEngineCrewSelectionSnapshot *crew_selection_data =
        crew_selection.empty() ? nullptr : crew_selection.data();
    const LcEngineCrewRoleSnapshot *crew_role_data =
        crew_roles.empty() ? nullptr : crew_roles.data();
    const LcEngineHudPlayerSnapshot *hud_player_data =
        hud_players.empty() ? nullptr : hud_players.data();
    const auto &surfaces = buffer.surface_raw;
    const LcEngineSurfaceSnapshot *surface_data =
        surfaces.empty() ? nullptr : surfaces.data();
    const size_t surface_count = surfaces.size();
    const auto &network_packets = buffer.network_packet_raw;
    const LcEngineNetworkPacketSnapshot *network_data =
        network_packets.empty() ? nullptr : network_packets.data();
    const size_t network_count = network_packets.size();
    const int32_t *known_owners =
        buffer.known_crew_owners.empty() ? nullptr : buffer.known_crew_owners.data();
    const int32_t *eliminated_owners = buffer.eliminated_crew_owners.empty()
        ? nullptr
        : buffer.eliminated_crew_owners.data();
    const uint64_t frame = static_cast<uint64_t>(game.FrameCounter);

    std::vector<std::string> control_inis;
    std::vector<const char *> control_ptrs;
    const auto control_it = g_frame_controls.find(frame);
    if (control_it != g_frame_controls.end()) {
        control_inis = std::move(control_it->second);
        g_frame_controls.erase(control_it);
        control_ptrs.reserve(control_inis.size());
        for (const std::string &entry : control_inis) {
            control_ptrs.push_back(entry.c_str());
        }
    }
    const char *const *control_data =
        control_ptrs.empty() ? nullptr : control_ptrs.data();
    const size_t control_count = control_ptrs.size();

    if (g_playback) {
        char *error_message = nullptr;
        if (!lc_engine_playback_compare(
                g_playback.get(),
                frame,
                object_data,
                raw.size(),
                global_effect_data,
                global_effects.size(),
                particle_data,
                particles.size(),
                crew_selection_data,
                crew_selection.size(),
                crew_role_data,
                crew_roles.size(),
                hud_player_data,
                hud_players.size(),
                surface_data,
                surface_count,
                network_data,
                network_count,
                control_data,
                control_count,
                known_owners,
                buffer.known_crew_owners.size(),
                eliminated_owners,
                buffer.eliminated_crew_owners.size(),
                &error_message)) {
            RustStringPtr error = MakeString(error_message);
            if (error) {
                LogError(std::string("Rust engine playback mismatch: ") + error.get());
            } else {
                LogError("Rust engine playback mismatch (no detail)");
            }
            g_disabled = true;
            g_playback.reset();
        }
    }

    if (g_recorder) {
        lc_engine_recorder_record(
            g_recorder.get(),
            frame,
            object_data,
            raw.size(),
            global_effect_data,
            global_effects.size(),
            particle_data,
            particles.size(),
            crew_selection_data,
            crew_selection.size(),
            crew_role_data,
            crew_roles.size(),
            known_owners,
            buffer.known_crew_owners.size(),
            eliminated_owners,
            buffer.eliminated_crew_owners.size(),
            hud_player_data,
            hud_players.size(),
            surface_data,
            surface_count,
            network_data,
            network_count,
            control_data,
            control_count);
    }

    if (g_runtime && !g_runtime_disabled) {
        if (g_runtime_authoritative) {
            char *error_message = nullptr;
            if (!lc_engine_runtime_advance_to_frame(
                    g_runtime.get(),
                    frame,
                    &error_message)) {
                RustStringPtr error = MakeString(error_message);
                if (error) {
                    LogError(std::string("Rust runtime advance failed: ") + error.get());
                } else {
                    LogError("Rust runtime advance failed (no detail)");
                }
                g_runtime.reset();
                g_runtime_disabled = true;
                ResetAuthoritativeLandscapeCache();
            } else {
                ApplyAuthoritativeRuntimeState(game);
            }
        } else {
            char *error_message = nullptr;
            if (!lc_engine_runtime_compare_snapshot(
                    g_runtime.get(),
                    frame,
                    object_data,
                    raw.size(),
                    global_effect_data,
                    global_effects.size(),
                    particle_data,
                    particles.size(),
                    crew_selection_data,
                    crew_selection.size(),
                    crew_role_data,
                    crew_roles.size(),
                    known_owners,
                    buffer.known_crew_owners.size(),
                    eliminated_owners,
                    buffer.eliminated_crew_owners.size(),
                    hud_player_data,
                    hud_players.size(),
                    surface_data,
                    surface_count,
                    network_data,
                    network_count,
                    control_data,
                    control_count,
                    RandomHold,
                    RandomCount,
                    &error_message)) {
                RustStringPtr error = MakeString(error_message);
                if (error) {
                    LogError(std::string("Rust runtime parity mismatch: ") + error.get());
                } else {
                    LogError("Rust runtime parity mismatch (no detail)");
                }
                g_runtime.reset();
                g_runtime_disabled = true;
                ResetAuthoritativeLandscapeCache();
            }
        }
        if (g_runtime && g_runtime_snapshot_enabled && g_runtime_snapshot_stream) {
            char *snapshot_error = nullptr;
            char *json_ptr =
                lc_engine_runtime_export_snapshot_json(g_runtime.get(), &snapshot_error);
            RustStringPtr error = MakeString(snapshot_error);
            RustStringPtr json = MakeString(json_ptr);
            if (!json) {
                if (error) {
                    LogWarning(
                        std::string("Failed to capture Rust runtime snapshot: ") + error.get());
                } else {
                    LogWarning("Failed to capture Rust runtime snapshot (no detail)");
                }
                g_runtime_snapshot_enabled = false;
            } else {
                g_runtime_snapshot_stream << json.get() << '\n';
                g_runtime_snapshot_stream.flush();
                if (!g_runtime_snapshot_stream.good()) {
                    LogWarning("Failed to write Rust runtime snapshot to stream");
                    g_runtime_snapshot_stream.close();
                    g_runtime_snapshot_enabled = false;
                }
            }
        }
    }
}

void Shutdown() {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_initialised) {
        return;
    }
    FinishPlayback();
    FlushRecording();
    ExportRuntimeState();
    g_playback.reset();
    ResetRecorder();
    g_initialised = false;
    g_disabled = false;
    g_frame_controls.clear();
    g_runtime.reset();
    g_runtime_disabled = false;
    ResetAuthoritativeLandscapeCache();
    if (g_runtime_snapshot_stream.is_open()) {
        g_runtime_snapshot_stream.close();
    }
    g_runtime_snapshot_enabled = false;
    g_runtime_snapshot_checked = false;
    ClearNetworkPacketLog();
    g_capture_network_packets.store(false, std::memory_order_release);
}

} // namespace RustEngineBridge

#endif // USE_RUST_ENGINE_VALIDATION
