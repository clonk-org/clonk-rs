#pragma once
#include <filesystem>
inline bool DirectoryExists(const char *path) { return std::filesystem::is_directory(path); }
