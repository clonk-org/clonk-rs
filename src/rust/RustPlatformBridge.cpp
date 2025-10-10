#include "RustPlatformBridge.h"

#ifdef USE_RUST_PLATFORM_PATHS

#include "lc_platform_ffi.h"

#include <optional>
#include <string>
#include <utility>

namespace {

using StringGetter = char *(*)();

std::optional<std::string> TakeString(char *raw) {
    if (!raw) {
        return std::nullopt;
    }
    std::string value(raw);
    lc_platform_string_free(raw);
    return value;
}

std::optional<std::string> Fetch(StringGetter getter) {
    return TakeString(getter());
}

} // namespace

namespace RustPlatformBridge {

std::optional<Paths> DiscoverPaths() {
    auto install_root = Fetch(&lc_platform_install_root);
    if (!install_root) {
        return std::nullopt;
    }
    auto planet_dir = Fetch(&lc_platform_planet_dir);
    if (!planet_dir) {
        return std::nullopt;
    }
    auto system_group = Fetch(&lc_platform_system_group_path);
    if (!system_group) {
        return std::nullopt;
    }
    auto user_data = Fetch(&lc_platform_user_data_dir);
    if (!user_data) {
        return std::nullopt;
    }
    auto cache = Fetch(&lc_platform_cache_dir);
    if (!cache) {
        return std::nullopt;
    }
    auto logs = Fetch(&lc_platform_logs_dir);
    if (!logs) {
        return std::nullopt;
    }
    auto temp = Fetch(&lc_platform_temp_dir);
    if (!temp) {
        return std::nullopt;
    }
    auto config = Fetch(&lc_platform_config_dir);
    if (!config) {
        return std::nullopt;
    }

    Paths paths;
    paths.install_root = std::move(*install_root);
    paths.planet_dir = std::move(*planet_dir);
    paths.system_group_path = std::move(*system_group);
    paths.user_data_dir = std::move(*user_data);
    paths.cache_dir = std::move(*cache);
    paths.logs_dir = std::move(*logs);
    paths.temp_dir = std::move(*temp);
    paths.config_dir = std::move(*config);
    return paths;
}

bool EnsureUserDirectories() {
    return lc_platform_ensure_user_dirs();
}

} // namespace RustPlatformBridge

#endif // USE_RUST_PLATFORM_PATHS
