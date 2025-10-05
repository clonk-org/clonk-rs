#include "RustConfigBridge.h"

#ifdef USE_RUST_CONFIG
#include "lc_config_ffi.h"

#include <mutex>
#include <string>

namespace {
std::mutex g_mutex;
ConfigHandle *g_handle = nullptr;
}

namespace RustConfigBridge {

bool LoadConfig(const std::string &path) {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (g_handle) {
        lc_config_free(g_handle);
        g_handle = nullptr;
    }
    g_handle = lc_config_load(path.c_str());
    return g_handle != nullptr;
}

std::optional<std::string> GetValue(const std::string &key) {
    return GetValueIn({}, key);
}

std::optional<std::string> GetValueIn(const std::string &section, const std::string &key) {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_handle) {
        return std::nullopt;
    }
    const char *raw = section.empty() ? lc_config_get_value(g_handle, key.c_str())
                                      : lc_config_get_value_in(g_handle, section.c_str(), key.c_str());
    if (!raw) {
        return std::nullopt;
    }
    std::string value(raw);
    lc_string_free(const_cast<char *>(raw));
    return value;
}

std::optional<std::string> Dump() {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_handle) {
        return std::nullopt;
    }
    const char *raw = lc_config_dump(g_handle);
    if (!raw) {
        return std::nullopt;
    }
    std::string dump(raw);
    lc_string_free(const_cast<char *>(raw));
    return dump;
}

std::optional<std::string> CompareWithLegacyDump(const std::string &legacy_dump) {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_handle) {
        return std::nullopt;
    }
    const char *raw = lc_config_compare_with_dump(g_handle, legacy_dump.c_str());
    if (!raw) {
        return std::nullopt;
    }
    std::string message(raw);
    lc_string_free(const_cast<char *>(raw));
    if (message.empty()) {
        return std::nullopt;
    }
    return message;
}

bool ReplaceFromText(const std::string &config_text) {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_handle) {
        return false;
    }
    return lc_config_replace_from_text(g_handle, config_text.c_str());
}

bool SaveConfig(const std::string &path) {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (!g_handle) {
        return false;
    }
    return lc_config_save(g_handle, path.c_str());
}

void Unload() {
    std::lock_guard<std::mutex> lock(g_mutex);
    if (g_handle) {
        lc_config_free(g_handle);
        g_handle = nullptr;
    }
}

} // namespace RustConfigBridge

#endif // USE_RUST_CONFIG
