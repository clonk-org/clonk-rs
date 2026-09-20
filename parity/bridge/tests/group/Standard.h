#pragma once
#include <cstring>
constexpr int _MAX_PATH = 4096;
constexpr int _MAX_FNAME = 256;
inline void SCopy(const char *source, char *target, int) { std::strcpy(target, source); }
inline void SAppend(const char *source, char *target, int) { std::strcat(target, source); }
inline void AppendBackslash(char *target) { std::strcat(target, "/"); }
