#pragma once

#include <optional>
#include <string>

namespace RustPlatformBridge {

struct Paths {
    std::string install_root;
    std::string planet_dir;
    std::string system_group_path;
    std::string user_data_dir;
    std::string cache_dir;
    std::string logs_dir;
    std::string temp_dir;
    std::string config_dir;
};

#ifdef USE_RUST_PLATFORM_PATHS
std::optional<Paths> DiscoverPaths();
bool EnsureUserDirectories();
#else
inline std::optional<Paths> DiscoverPaths() {
    return std::nullopt;
}

inline bool EnsureUserDirectories() {
    return false;
}
#endif

} // namespace RustPlatformBridge
