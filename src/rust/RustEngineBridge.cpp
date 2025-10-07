#include "RustEngineBridge.h"

#ifdef USE_RUST_ENGINE_VALIDATION

#include "lc_engine_ffi.h"

#include <C4Include.h>
#include <C4Game.h>
#include <C4Log.h>
#include <C4Object.h>
#include <C4ObjectList.h>
#include <C4Effects.h>

#include <Fixed.h>

#include <algorithm>
#include <cstdlib>
#include <fstream>
#include <memory>
#include <mutex>
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

struct SnapshotBuffer {
    std::vector<SnapshotEntry> entries;
    std::vector<LcEngineObjectSnapshot> raw;
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
    const uint64_t frame = static_cast<uint64_t>(game.FrameCounter);

    if (g_playback) {
        char *error_message = nullptr;
        if (!lc_engine_playback_compare(g_playback.get(), frame, raw.data(), raw.size(), &error_message)) {
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
        lc_engine_recorder_record(g_recorder.get(), frame, raw.data(), raw.size());
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
