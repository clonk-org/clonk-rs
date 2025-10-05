#pragma once

#include <optional>
#include <string>

namespace RustConfigBridge {
#ifdef USE_RUST_CONFIG
bool LoadConfig(const std::string &path);
std::optional<std::string> GetValue(const std::string &key);
std::optional<std::string> GetValueIn(const std::string &section, const std::string &key);
std::optional<std::string> Dump();
std::optional<std::string> CompareWithLegacyDump(const std::string &legacy_dump);
bool ReplaceFromText(const std::string &config_text);
bool SaveConfig(const std::string &path);
void Unload();
#else
inline bool LoadConfig(const std::string &) { return false; }
inline std::optional<std::string> GetValue(const std::string &) { return std::nullopt; }
inline std::optional<std::string> GetValueIn(const std::string &, const std::string &) { return std::nullopt; }
inline std::optional<std::string> Dump() { return std::nullopt; }
inline std::optional<std::string> CompareWithLegacyDump(const std::string &) { return std::nullopt; }
inline bool ReplaceFromText(const std::string &) { return false; }
inline bool SaveConfig(const std::string &) { return false; }
inline void Unload() {}
#endif
} // namespace RustConfigBridge
