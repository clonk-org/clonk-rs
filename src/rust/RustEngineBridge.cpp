#include "RustEngineBridge.h"

#ifdef USE_RUST_ENGINE_VALIDATION

#include "lc_engine_ffi.h"

#include <C4Include.h>
#include <C4Game.h>
#include <C4Log.h>
#include <C4Object.h>
#include <C4ObjectList.h>
#include <C4Effects.h>
#include <C4Player.h>

#include <Fixed.h>

#include <algorithm>
#include <cstdlib>
#include <fstream>
#include <memory>
#include <mutex>
#include <set>
#include <string>
#include <vector>

namespace {

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

using RecorderPtr = std::unique_ptr<LcEngineRecorderHandle, RecorderDeleter>;
using PlaybackPtr = std::unique_ptr<LcEnginePlaybackHandle, PlaybackDeleter>;
using RustStringPtr = std::unique_ptr<char, decltype(&lc_engine_string_free)>;

struct SnapshotEntry {
    LcEngineObjectSnapshot snapshot{};
    std::string definition;
    std::string action;
    std::vector<LcEngineEffectSnapshot> effects;
    std::vector<std::string> effect_names;
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

struct SnapshotBuffer {
    std::vector<SnapshotEntry> entries;
    std::vector<LcEngineObjectSnapshot> raw;
    std::vector<LcEngineEffectSnapshot> global_effects;
    std::vector<std::string> global_effect_names;
    std::vector<CrewSelectionEntry> crew_selections;
    std::vector<LcEngineCrewSelectionSnapshot> crew_selection_raw;
    std::vector<CrewRoleEntry> crew_roles;
    std::vector<LcEngineCrewRoleSnapshot> crew_role_raw;
    std::vector<int32_t> known_crew_owners;
    std::vector<int32_t> eliminated_crew_owners;
};

std::mutex g_mutex;
bool g_initialised = false;
bool g_disabled = false;
RecorderPtr g_recorder;
PlaybackPtr g_playback;
std::string g_record_path;

RustStringPtr MakeString(char *raw) {
    return RustStringPtr(raw, lc_engine_string_free);
}

void LogWarning(const std::string &message) {
    LogNTr(spdlog::level::warn, message);
}

void LogError(const std::string &message) {
    LogNTr(spdlog::level::err, message);
}

std::string LoadFile(const std::string &path) {
    std::ifstream stream(path);
    if (!stream) {
        return {};
    }
    return std::string(std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>());
}

SnapshotBuffer CollectSnapshotBuffer(C4Game &game) {
    SnapshotBuffer buffer;
    std::set<int32_t> active_owners;
    for (auto it = game.Objects.begin(); it != game.Objects.end(); ++it) {
        C4Object *object = *it;
        if (!object || !object->Status) {
            continue;
        }
        SnapshotEntry entry;
        entry.snapshot.id = static_cast<uint64_t>(object->Number);
        entry.definition = object->Def ? object->Def->GetName() : "";
        entry.snapshot.position_x = fixtoi(object->fix_x);
        entry.snapshot.position_y = fixtoi(object->fix_y);
        entry.snapshot.velocity_x = fixtoi(object->xdir);
        entry.snapshot.velocity_y = fixtoi(object->ydir);
        entry.snapshot.energy = object->Energy;
        entry.snapshot.owner = static_cast<int32_t>(object->Owner);
        entry.snapshot.crew_member = (object->OCF & OCF_CrewMember) != 0;
        entry.action = object->Action.Name;
        entry.snapshot.action_name = entry.action.c_str();
        entry.snapshot.action_phase = object->Action.Phase;

        if (entry.snapshot.crew_member && entry.snapshot.owner != NO_OWNER) {
            active_owners.insert(entry.snapshot.owner);
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
        buffer.raw.push_back(entry.snapshot);
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

    const char *record_path = std::getenv("LC_RUST_ENGINE_RECORD");
    if (record_path && *record_path) {
        InitialiseRecorder(record_path);
    }

    if (const char *playback_path = std::getenv("LC_RUST_ENGINE_PLAYBACK")) {
        if (*playback_path) {
            std::string json = LoadFile(playback_path);
            if (!InitialisePlayback(json)) {
                g_disabled = true;
            }
        }
    }

    if (!g_recorder && !g_playback) {
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

} // namespace

namespace RustEngineBridge {

bool IsActive() {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    return !g_disabled && (g_recorder || g_playback);
}

void OnFrame(C4Game &game) {
    std::lock_guard<std::mutex> lock(g_mutex);
    EnsureInitialised();
    if (g_disabled) {
        return;
    }

    SnapshotBuffer buffer = CollectSnapshotBuffer(game);
    const auto &raw = buffer.raw;
    const auto &global_effects = buffer.global_effects;
    const auto &crew_selection = buffer.crew_selection_raw;
    const auto &crew_roles = buffer.crew_role_raw;
    const LcEngineObjectSnapshot *object_data = raw.empty() ? nullptr : raw.data();
    const LcEngineEffectSnapshot *global_effect_data =
        global_effects.empty() ? nullptr : global_effects.data();
    const LcEngineCrewSelectionSnapshot *crew_selection_data =
        crew_selection.empty() ? nullptr : crew_selection.data();
    const LcEngineCrewRoleSnapshot *crew_role_data =
        crew_roles.empty() ? nullptr : crew_roles.data();
    const int32_t *known_owners =
        buffer.known_crew_owners.empty() ? nullptr : buffer.known_crew_owners.data();
    const int32_t *eliminated_owners = buffer.eliminated_crew_owners.empty()
        ? nullptr
        : buffer.eliminated_crew_owners.data();
    const uint64_t frame = static_cast<uint64_t>(game.FrameCounter);

    if (g_playback) {
        char *error_message = nullptr;
        if (!lc_engine_playback_compare(
                g_playback.get(),
                frame,
                object_data,
                raw.size(),
                global_effect_data,
                global_effects.size(),
                crew_selection_data,
                crew_selection.size(),
                crew_role_data,
                crew_roles.size(),
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
            crew_selection_data,
            crew_selection.size(),
            crew_role_data,
            crew_roles.size(),
            known_owners,
            buffer.known_crew_owners.size(),
            eliminated_owners,
            buffer.eliminated_crew_owners.size());
    }
}

void Shutdown() {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_initialised) {
        return;
    }
    FinishPlayback();
    FlushRecording();
    g_playback.reset();
    ResetRecorder();
    g_initialised = false;
    g_disabled = false;
}

} // namespace RustEngineBridge

#endif // USE_RUST_ENGINE_VALIDATION
